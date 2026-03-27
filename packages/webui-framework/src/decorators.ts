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

/** Convert camelCase to kebab-case for attribute reflection. */
function toKebabCase(str: string): string {
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

  Object.defineProperty(proto, name, {
    get(this: Record<string, unknown>) {
      return this[backingKey];
    },
    set(this: Record<string, unknown>, newValue: unknown) {
      const oldValue = this[backingKey];
      if (oldValue === newValue) return;
      this[backingKey] = newValue;

      // Notify the class-level change callback if it exists.
      const cb = (this as Record<string, unknown>)[`${name}Changed`];
      if (typeof cb === 'function') {
        (cb as (old: unknown, next: unknown) => void).call(this, oldValue, newValue);
      }

      // Trigger a reactive update when the element is connected.
      if (typeof (this as Record<string, unknown>)['$update'] === 'function') {
        if ((this as unknown as HTMLElement).isConnected) {
          (this as { $update(path?: string): void }).$update(name);
        }
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
export function getObservableNames(ctor: Function): Set<string> {
  return observableRegistry.get(ctor) ?? new Set();
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
        (this as Record<string, unknown>)[propName] = newVal;
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

// ---------------------------------------------------------------------------
// @volatile
// ---------------------------------------------------------------------------

/**
 * Marks a getter as volatile — re-evaluated every time it is accessed.
 */
export function volatile(
  target: object,
  name: string,
  descriptor: PropertyDescriptor,
): PropertyDescriptor {
  return {
    ...descriptor,
    enumerable: false,
    configurable: true,
  };
}

