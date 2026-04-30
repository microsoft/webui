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
function createReactiveProperty(
  proto: Record<string, unknown>,
  name: string,
): void {
  const backingKey = `_${name}`;
  const changedKey = `${name}Changed`;

  Object.defineProperty(proto, name, {
    get(this: Record<string, unknown>) {
      return this[backingKey];
    },
    set(this: Record<string, unknown>, newValue: unknown) {
      const oldValue = this[backingKey];
      if (oldValue === newValue) return;
      this[backingKey] = newValue;

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

/**
 * Marks a property as observable. When the value changes the decorator will:
 * 1. Call `this.<prop>Changed(oldValue, newValue)` if defined.
 * 2. Call `this.$update(name)` if the element is connected, targeting
 *    only bindings that reference this property.
 */
export function observable(target: object, name: string): void {
  const ctor = (target as Record<string, unknown>).constructor as Function;
  if (!observableRegistry.has(ctor)) {
    observableRegistry.set(ctor, new Set());
  }
  observableRegistry.get(ctor)!.add(name);
  createReactiveProperty(target as Record<string, unknown>, name);
}

// ---------------------------------------------------------------------------
// @attr
// ---------------------------------------------------------------------------

/**
 * Per-class attribute metadata: maps each observed HTML attribute name to
 * its backing property name and boolean-mode flag. A single WeakMap entry
 * per class replaces the older split between attribute-routing and
 * boolean-typing registries — `attributeChangedCallback` resolves both
 * with one Map lookup.
 */
interface AttrEntry {
  prop: string;
  bool: boolean;
}
const attrMeta = new WeakMap<Function, Map<string, AttrEntry>>();

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
  const isBool = options?.mode === 'boolean';
  const backingKey = `_${name}`;
  const changedKey = `${name}Changed`;

  // Install a dedicated getter/setter that inlines property → host attribute
  // reflection. We don't share `createReactiveProperty` here because the
  // reflection logic must run on every set and inlining it avoids both an
  // extra closure allocation per @attr and an indirect call per setter
  // invocation. The subsequent attributeChangedCallback re-entry is a
  // no-op thanks to the `oldValue === newValue` guard.
  Object.defineProperty(proto, name, {
    get(this: Record<string, unknown>) {
      return this[backingKey];
    },
    set(this: Record<string, unknown>, newValue: unknown) {
      const oldValue = this[backingKey];
      if (oldValue === newValue) return;
      this[backingKey] = newValue;

      const cb = this[changedKey];
      if (typeof cb === 'function') {
        (cb as (old: unknown, next: unknown) => void).call(this, oldValue, newValue);
      }

      // Skip DOM work when the element is not in a document. The HTML
      // spec forbids custom-element constructors from "gain[ing] any
      // attributes or children", and class-field initializers
      // (`@attr foo = 'x'`) run inside the constructor — guarding on
      // `isConnected` keeps that legal. Property writes performed on
      // a detached element therefore don't reflect to the attribute
      // until something else triggers a setter while connected.
      const host = this as unknown as Element;
      if (!host.isConnected) return;

      if (isBool) {
        const want = !!newValue;
        if (want !== host.hasAttribute(attrName)) {
          if (want) host.setAttribute(attrName, '');
          else host.removeAttribute(attrName);
        }
      } else if (newValue == null) {
        if (host.hasAttribute(attrName)) host.removeAttribute(attrName);
      } else {
        const s = typeof newValue === 'string' ? newValue : String(newValue);
        if (host.getAttribute(attrName) !== s) host.setAttribute(attrName, s);
      }

      const upd = this['$update'] as ((path?: string) => void) | undefined;
      if (upd) upd.call(this, name);
    },
    enumerable: true,
    configurable: true,
  });

  // Register attribute metadata in the per-class map.
  let meta = attrMeta.get(ctor);
  if (!meta) {
    meta = new Map();
    attrMeta.set(ctor, meta);
  }
  meta.set(attrName, { prop: name, bool: isBool });

  // Wire observedAttributes + attributeChangedCallback once per class.
  if (!ctor._observedAttrs) {
    ctor._observedAttrs = [];

    Object.defineProperty(ctor, 'observedAttributes', {
      get() {
        return ctor._observedAttrs ?? [];
      },
      configurable: true,
    });

    const origACB = proto['attributeChangedCallback'] as
      | ((name: string, oldVal: string | null, newVal: string | null) => void)
      | undefined;

    proto['attributeChangedCallback'] = function (
      this: Record<string, unknown>,
      attribute: string,
      oldVal: string | null,
      newVal: string | null,
    ) {
      const entry = attrMeta.get(this.constructor as Function)?.get(attribute);
      if (entry) {
        (this as Record<string, unknown>)[entry.prop] = entry.bool ? newVal !== null : newVal;
      }
      if (origACB) origACB.call(this, attribute, oldVal, newVal);
    };
  }

  ctor._observedAttrs.push(attrName);
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
