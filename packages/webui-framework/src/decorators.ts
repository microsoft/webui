// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Reactive decorators for WebUIElement properties.
 *
 * Uses TypeScript's `experimentalDecorators` emit, matching the FAST ecosystem
 * conventions. This module holds both the reactive metadata registries (the
 * read side that {@link WebUIElement} consults to route `setState`/SSR seeding)
 * and the `@observable`/`@attr` decorators (the write side that populates them).
 *
 * Note on bundling: esbuild — the bundler every WebUI app and example uses —
 * performs function-level dead-code elimination, so an HTML-only app that never
 * references `@observable`/`@attr` already tree-shakes the write side away while
 * keeping only the read helpers the engine imports. Splitting this module would
 * add files without removing a single shipped byte, so it deliberately stays
 * one unit.
 */

// ---------------------------------------------------------------------------
// kebab-case attribute naming
// ---------------------------------------------------------------------------

/**
 * Map of camelCase property names to their HTML attribute names.
 *
 * ARIA attributes (`ariaXxxYyy → aria-` + lowercase remainder) are handled
 * algorithmically in `toKebabCase`. Only HTML global/element attributes
 * with irregular mappings (concatenated lowercase) need explicit entries.
 */
const propertyToAttribute: Record<string, string> = Object.assign(Object.create(null) as Record<string, string>, {
  accessKey: 'accesskey',
  autoCapitalize: 'autocapitalize',
  contentEditable: 'contenteditable',
  crossOrigin: 'crossorigin',
  dirName: 'dirname',
  fetchPriority: 'fetchpriority',
  formAction: 'formaction',
  formEnctype: 'formenctype',
  formMethod: 'formmethod',
  formNoValidate: 'formnovalidate',
  formTarget: 'formtarget',
  inputMode: 'inputmode',
  isMap: 'ismap',
  maxLength: 'maxlength',
  minLength: 'minlength',
  noModule: 'nomodule',
  noValidate: 'novalidate',
  readOnly: 'readonly',
  referrerPolicy: 'referrerpolicy',
  tabIndex: 'tabindex',
  useMap: 'usemap',
});

/**
 * Convert a camelCase DOM property name into its kebab-case HTML attribute form.
 *
 * Optimized for framework-level hot paths where attribute normalization may run
 * thousands of times per render. It performs three progressively cheaper checks:
 * a direct lookup for irregular mappings, a fast path for ARIA attributes, then
 * a tight ASCII-only scan. No regex engines, callbacks, or match objects.
 */
export function toKebabCase(str: string): string {
  const mapped = propertyToAttribute[str];
  if (mapped) return mapped;
  // ARIA properties: ariaXxxYyy → aria- + lowercase remainder
  if (str.length > 4 && str.charCodeAt(0) === 97 /* a */ && str.startsWith('aria') && str.charCodeAt(4) >= 65 && str.charCodeAt(4) <= 90) {
    return 'aria-' + str.slice(4).toLowerCase();
  }
  let out = '';
  for (let i = 0; i < str.length; i++) {
    const code = str.charCodeAt(i);
    out += code >= 65 && code <= 90 ? '-' + String.fromCharCode(code + 32) : str[i];
  }
  return out;
}

// ---------------------------------------------------------------------------
// Reactive metadata registries (read side)
// ---------------------------------------------------------------------------

type ReactiveInstance = Record<string | symbol, unknown>;

interface AttrDefinition {
  attribute: string;
  property: string;
  boolean: boolean;
}

/** Marks the attribute currently being reflected so the reverse
 *  attributeChangedCallback path does not echo it back into the property. */
const reflectingAttribute = Symbol('webui.reflectingAttribute');

const EMPTY_SET: Set<string> = Object.freeze(new Set<string>()) as Set<string>;

function parentConstructor(ctor: Function): Function | null {
  const parent = Object.getPrototypeOf(ctor);
  return typeof parent === 'function' && parent !== Function.prototype ? parent : null;
}

/** Per-class registry of @observable property names. */
const observableRegistry = new WeakMap<Function, Set<string>>();

/** Get the set of @observable property names registered for a class. */
export function getObservableNames(ctor: Function): Set<string> {
  const names = observableRegistry.get(ctor);
  if (names) return names;

  const parent = parentConstructor(ctor);
  return parent ? getObservableNames(parent) : EMPTY_SET;
}

function registerObservableProperty(ctor: Function, name: string): void {
  let names = observableRegistry.get(ctor);
  if (!names) {
    const parent = parentConstructor(ctor);
    const inherited = parent ? getObservableNames(parent) : EMPTY_SET;
    names = inherited.size > 0 ? new Set(inherited) : new Set();
    observableRegistry.set(ctor, names);
  }
  names.add(name);
}

/** Registry of attribute-name → property-name mappings per constructor.
 *  Used by `attributeChangedCallback` to route attribute changes to properties. */
const attrByAttribute = new WeakMap<Function, Map<string, AttrDefinition>>();

/** Registry of property-name → attribute metadata, used for mount-time sync. */
const attrByProperty = new WeakMap<Function, Map<string, AttrDefinition>>();

function inheritedAttrMap(
  registry: WeakMap<Function, Map<string, AttrDefinition>>,
  ctor: Function,
): Map<string, AttrDefinition> | undefined {
  let current = parentConstructor(ctor);
  while (current) {
    const map = registry.get(current);
    if (map) return map;
    current = parentConstructor(current);
  }
  return undefined;
}

function attrDefinitionFor(
  ctor: Function,
  attribute: string,
): AttrDefinition | undefined {
  let current: Function | null = ctor;
  while (current) {
    const definition = attrByAttribute.get(current)?.get(attribute);
    if (definition) return definition;
    current = parentConstructor(current);
  }
  return undefined;
}

function attrPropertyMapFor(ctor: Function): Map<string, AttrDefinition> | undefined {
  let current: Function | null = ctor;
  while (current) {
    const map = attrByProperty.get(current);
    if (map) return map;
    current = parentConstructor(current);
  }
  return undefined;
}

export function isAttributeProperty(ctor: Function, property: string): boolean {
  return attrPropertyMapFor(ctor)?.has(property) === true;
}

// ---------------------------------------------------------------------------
// Attribute reflection
// ---------------------------------------------------------------------------

function setReflectingAttribute(instance: ReactiveInstance, attrName: string): void {
  instance[reflectingAttribute] = attrName;
}

function restoreReflectingAttribute(instance: ReactiveInstance): void {
  instance[reflectingAttribute] = undefined;
}

function reflectPropertyToAttribute(
  instance: ReactiveInstance,
  definition: AttrDefinition,
  value: unknown,
): void {
  const element = instance as unknown as HTMLElement;
  const attrName = definition.attribute;

  if (definition.boolean) {
    const shouldHaveAttribute = Boolean(value);
    if (element.hasAttribute(attrName) === shouldHaveAttribute) return;
    setReflectingAttribute(instance, attrName);
    try {
      if (shouldHaveAttribute) element.setAttribute(attrName, '');
      else element.removeAttribute(attrName);
    } finally {
      restoreReflectingAttribute(instance);
    }
    return;
  }

  if (value == null) {
    if (!element.hasAttribute(attrName)) return;
    setReflectingAttribute(instance, attrName);
    try {
      element.removeAttribute(attrName);
    } finally {
      restoreReflectingAttribute(instance);
    }
    return;
  }

  const attrValue = typeof value === 'string' ? value : String(value);
  if (element.getAttribute(attrName) === attrValue) return;
  setReflectingAttribute(instance, attrName);
  try {
    element.setAttribute(attrName, attrValue);
  } finally {
    restoreReflectingAttribute(instance);
  }
}

/** Reflect every @attr property of an instance to its attribute at mount. */
export function syncAttrProperties(instance: object, ctor: Function): void {
  const attrs = attrPropertyMapFor(ctor);
  if (!attrs) return;

  const reactiveInstance = instance as ReactiveInstance;
  for (const definition of attrs.values()) {
    reflectPropertyToAttribute(
      reactiveInstance,
      definition,
      reactiveInstance[definition.property],
    );
  }
}

// ---------------------------------------------------------------------------
// Decorators (write side)
// ---------------------------------------------------------------------------

/**
 * Shared logic for installing a reactive getter/setter on a class prototype.
 * The backing value is stored in a private `_prop` field on the instance.
 */
function createReactiveProperty(
  proto: Record<string, unknown>,
  name: string,
  attrDefinition?: AttrDefinition,
): void {
  const backingKey = `_${name}`;
  const changedKey = `${name}Changed`;

  Object.defineProperty(proto, name, {
    get(this: ReactiveInstance) {
      return this[backingKey];
    },
    set(this: ReactiveInstance, newValue: unknown) {
      const oldValue = this[backingKey];
      if (Object.is(oldValue, newValue)) return;
      this[backingKey] = newValue;

      if (attrDefinition && this['$ready'] === true) {
        reflectPropertyToAttribute(this, attrDefinition, newValue);
      }

      const cb = this[changedKey];
      if (typeof cb === 'function') {
        (cb as (old: unknown, next: unknown) => void).call(this, oldValue, newValue);
      }

      if ((this as unknown as HTMLElement).isConnected) {
        const upd = this['$update'] as ((path?: string) => void) | undefined;
        if (upd) upd.call(this, name);
      }
    },
    enumerable: true,
    configurable: true,
  });
}

/**
 * Marks a property as observable. When the value changes the decorator will:
 * 1. Call `this.<prop>Changed(oldValue, newValue)` if defined.
 * 2. Call `this.$update(name)` if the element is connected, targeting
 *    only bindings that reference this property.
 */
export function observable(target: object, name: string): void {
  const ctor = (target as Record<string, unknown>).constructor as Function;
  registerObservableProperty(ctor, name);
  createReactiveProperty(target as Record<string, unknown>, name);
}

/**
 * Like {@link observable} but also reflects to/from an HTML attribute
 * (kebab-case). The decorator patches `observedAttributes` and
 * `attributeChangedCallback` on the class so changes flow in both directions.
 *
 * @example
 * ```ts
 * class MyEl extends WebUIElement {
 *   @attr myProp = 'default';
 *   // syncs with attribute "my-prop"
 * }
 * ```
 */
export interface AttrOptions {
  attribute?: string;
  /** When `'boolean'`, the property is `true` when the attribute is present
   *  and `false` when absent — matching native HTML boolean attribute semantics.
   *  Default is string mode (property receives the attribute string value). */
  mode?: 'boolean';
}

function applyAttr(
  target: object,
  name: string,
  options?: AttrOptions,
): void {
  const proto = target as Record<string, unknown>;
  const ctor = proto.constructor as typeof HTMLElement & {
    _observedAttrs?: string[];
  };
  const attrName = options?.attribute ?? toKebabCase(name);
  const definition: AttrDefinition = {
    attribute: attrName,
    property: name,
    boolean: options?.mode === 'boolean',
  };

  // 1. Install the reactive getter/setter (same as @observable), with
  // attribute reflection enabled after the element finishes hydration.
  registerObservableProperty(ctor, name);
  createReactiveProperty(proto, name, definition);

  // 2. Register the attribute mapping.
  let byAttribute = attrByAttribute.get(ctor);
  if (!byAttribute) {
    const inherited = inheritedAttrMap(attrByAttribute, ctor);
    byAttribute = inherited ? new Map(inherited) : new Map();
    attrByAttribute.set(ctor, byAttribute);
  }
  byAttribute.set(attrName, definition);

  let byProperty = attrByProperty.get(ctor);
  if (!byProperty) {
    const inherited = inheritedAttrMap(attrByProperty, ctor);
    byProperty = inherited ? new Map(inherited) : new Map();
    attrByProperty.set(ctor, byProperty);
  }
  byProperty.set(name, definition);

  // 3. Accumulate observed attributes on the constructor.
  if (!Object.prototype.hasOwnProperty.call(ctor, '_observedAttrs')) {
    const inheritedAttrs = ctor._observedAttrs;
    ctor._observedAttrs = inheritedAttrs ? inheritedAttrs.slice() : [];

    // Define the static getter that `customElements.define` inspects.
    Object.defineProperty(ctor, 'observedAttributes', {
      get() {
        return ctor._observedAttrs ?? [];
      },
      configurable: true,
    });

    // Patch `attributeChangedCallback` once per class.
    const origACB = proto['attributeChangedCallback'] as
      | ((name: string, oldVal: string | null, newVal: string | null) => void)
      | undefined;

    proto['attributeChangedCallback'] = function (
      this: Record<string, unknown>,
      attribute: string,
      oldVal: string | null,
      newVal: string | null,
    ) {
      // Route the attribute change to the corresponding property.
      const definition = attrDefinitionFor(this.constructor as Function, attribute);
      if (
        definition !== undefined &&
        (this as ReactiveInstance)[reflectingAttribute] !== attribute
      ) {
        (this as Record<string, unknown>)[definition.property] = definition.boolean
          ? newVal !== null
          : newVal;
      }

      // Preserve any pre-existing attributeChangedCallback.
      if (origACB) {
        origACB.call(this, attribute, oldVal, newVal);
      }
    };
  }

  ctor._observedAttrs!.push(attrName);
}

export function attr(target: object, name: string): void;
export function attr(options: AttrOptions): (target: object, name: string) => void;
export function attr(
  targetOrOptions: object | AttrOptions,
  name?: string,
): void | ((target: object, name: string) => void) {
  if (typeof name === 'string') {
    applyAttr(targetOrOptions as object, name);
    return;
  }

  const options = targetOrOptions as AttrOptions;
  return (target: object, propName: string): void => {
    applyAttr(target, propName, options);
  };
}
