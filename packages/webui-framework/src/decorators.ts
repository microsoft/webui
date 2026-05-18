// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Reactive decorators for WebUIElement properties.
 *
 * Uses legacy/experimental TypeScript decorators (`experimentalDecorators: true`)
 * for compatibility with the FAST ecosystem conventions.
 */

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/**
 * Map of camelCase property names to their HTML attribute names.
 *
 * ARIA attributes (`ariaXxxYyy → aria-` + lowercase remainder) are handled
 * algorithmically in `toKebabCase`. Only HTML global/element attributes
 * with irregular mappings (concatenated lowercase) need explicit entries.
 */
const propertyToAttribute: Record<string, string> = Object.assign(Object.create(null) as Record<string, string>, {
  // --- HTML global/element attributes ---
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
 * This function is optimized for framework-level hot paths where attribute
 * normalization may run thousands of times per render. It performs three
 * progressively cheaper checks:
 *
 * 1. **Direct lookup for irregular mappings**  
 *    Many DOM properties (e.g., `readOnly`, `tabIndex`, `crossOrigin`) do not
 *    follow simple camelCase → kebab-case rules. These are resolved through a
 *    precomputed `propertyToAttribute` map for O(1) returns with no string
 *    processing.
 *
 * 2. **Fast path for ARIA attributes**  
 *    ARIA properties always begin with `aria` followed by an uppercase letter
 *    (e.g., `ariaDescribedBy`). These map to `aria-` + the lowercase remainder.
 *    This branch avoids the general loop and uses the engine-optimized
 *    `.toLowerCase()` for the suffix.
 *
 * 3. **General camelCase → kebab-case conversion**  
 *    For all other inputs, the function performs a tight ASCII-only scan:
 *    uppercase A–Z (65–90) are converted to lowercase and prefixed with `-`,
 *    while all other characters are copied as-is. This avoids regex engines,
 *    callback allocations, and match objects, producing predictable,
 *    allocation-minimal performance ideal for DOM attribute reflection.
 *
 * The result is a predictable, JIT-friendly transformation suitable for
 * attribute diffing, SSR serialization, and runtime DOM patching.
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

/**
 * Shared logic for installing a reactive getter/setter on a class prototype.
 * The backing value is stored in a private `_prop` field on the instance.
 */
interface AttrDefinition {
  attribute: string;
  property: string;
  boolean: boolean;
}

type ReactiveInstance = Record<string | symbol, unknown>;

const reflectingAttribute = Symbol('webui.reflectingAttribute');

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

function setReflectingAttribute(instance: ReactiveInstance, attrName: string): void {
  instance[reflectingAttribute] = attrName;
}

function restoreReflectingAttribute(instance: ReactiveInstance): void {
  instance[reflectingAttribute] = undefined;
}

// ---------------------------------------------------------------------------
// @observable
// ---------------------------------------------------------------------------

/** Per-class registry of @observable property names. */
const observableRegistry = new WeakMap<Function, Set<string>>();

/**
 * Get the set of @observable property names registered for a class.
 */
const EMPTY_SET: Set<string> = Object.freeze(new Set<string>()) as Set<string>;

export function getObservableNames(ctor: Function): Set<string> {
  return observableRegistry.get(ctor) ?? EMPTY_SET;
}

function registerObservableProperty(ctor: Function, name: string): void {
  let names = observableRegistry.get(ctor);
  if (!names) {
    names = new Set();
    observableRegistry.set(ctor, names);
  }
  names.add(name);
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

// ---------------------------------------------------------------------------
// @attr
// ---------------------------------------------------------------------------

/**
 * Registry of attribute-name → property-name mappings per constructor.
 * Used by `attributeChangedCallback` to route attribute changes to properties.
 */
const attrByAttribute = new WeakMap<Function, Map<string, AttrDefinition>>();

/** Registry of property-name → attribute metadata, used for mount-time sync. */
const attrByProperty = new WeakMap<Function, Map<string, AttrDefinition>>();

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
    byAttribute = new Map();
    attrByAttribute.set(ctor, byAttribute);
  }
  byAttribute.set(attrName, definition);

  let byProperty = attrByProperty.get(ctor);
  if (!byProperty) {
    byProperty = new Map();
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
      const map = attrByAttribute.get(this.constructor as Function);
      const definition = map?.get(attribute);
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

export function syncAttrProperties(
  instance: object,
  ctor: Function,
): void {
  const attrs = attrByProperty.get(ctor);
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
