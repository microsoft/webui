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
 * Covers two categories of irregular mappings:
 *
 * 1. Multi-word ARIA attributes — concatenated lowercase after `aria-`
 *    (e.g., `ariaDescribedBy` → `aria-describedby`), per the ARIAMixin spec.
 * 2. HTML global/element attributes — concatenated lowercase attribute names
 *    with camelCase property counterparts (e.g., `readOnly` → `readonly`).
 */
const propertyToAttribute: Record<string, string> = {
  // --- ARIA (ARIAMixin) ---
  ariaActiveDescendant: 'aria-activedescendant',
  ariaAutoComplete: 'aria-autocomplete',
  ariaBrailleLabel: 'aria-braillelabel',
  ariaBrailleRoleDescription: 'aria-brailleroledescription',
  ariaColCount: 'aria-colcount',
  ariaColIndex: 'aria-colindex',
  ariaColIndexText: 'aria-colindextext',
  ariaColSpan: 'aria-colspan',
  ariaDescribedBy: 'aria-describedby',
  ariaDropEffect: 'aria-dropeffect',
  ariaErrorMessage: 'aria-errormessage',
  ariaFlowTo: 'aria-flowto',
  ariaHasPopup: 'aria-haspopup',
  ariaKeyShortcuts: 'aria-keyshortcuts',
  ariaLabelledBy: 'aria-labelledby',
  ariaMultiLine: 'aria-multiline',
  ariaMultiSelectable: 'aria-multiselectable',
  ariaPosInSet: 'aria-posinset',
  ariaReadOnly: 'aria-readonly',
  ariaRoleDescription: 'aria-roledescription',
  ariaRowCount: 'aria-rowcount',
  ariaRowIndex: 'aria-rowindex',
  ariaRowIndexText: 'aria-rowindextext',
  ariaRowSpan: 'aria-rowspan',
  ariaSetSize: 'aria-setsize',
  ariaValueMax: 'aria-valuemax',
  ariaValueMin: 'aria-valuemin',
  ariaValueNow: 'aria-valuenow',
  ariaValueText: 'aria-valuetext',
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
};

/**
 * Convert camelCase to kebab-case for attribute reflection.
 *
 * Multi-word ARIA properties are handled via a lookup table so that
 * `ariaDescribedBy` correctly maps to `aria-describedby` rather than
 * the naive `aria-described-by`.
 */
export function toKebabCase(str: string): string {
  if (str in propertyToAttribute) return propertyToAttribute[str];
  return str.replace(/[A-Z]/g, (m) => `-${m.toLowerCase()}`);
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
 * Registry of attribute-name → property-name mappings per constructor.
 * Used by `attributeChangedCallback` to route attribute changes to properties.
 */
const attrMap = new WeakMap<Function, Map<string, string>>();

/** Registry of boolean-mode attribute names per constructor. */
const boolAttrs = new WeakMap<Function, Set<string>>();

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

  // 1. Install the reactive getter/setter (same as @observable).
  createReactiveProperty(proto, name);

  // 2. Register the attribute mapping.
  const attrName = options?.attribute ?? toKebabCase(name);
  if (!attrMap.has(ctor)) {
    attrMap.set(ctor, new Map());
  }
  attrMap.get(ctor)!.set(attrName, name);

  // Track boolean-mode attrs.
  if (options?.mode === 'boolean') {
    if (!boolAttrs.has(ctor)) {
      boolAttrs.set(ctor, new Set());
    }
    boolAttrs.get(ctor)!.add(attrName);
  }

  // 3. Accumulate observed attributes on the constructor.
  if (!ctor._observedAttrs) {
    ctor._observedAttrs = [];

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
      const map = attrMap.get(this.constructor as Function);
      const propName = map?.get(attribute);
      if (propName !== undefined) {
        const isBool = boolAttrs.get(this.constructor as Function)?.has(attribute);
        (this as Record<string, unknown>)[propName] = isBool ? newVal !== null : newVal;
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
