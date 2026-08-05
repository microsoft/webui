// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

/**
 * `WebUIElement extends TemplateElement extends HTMLElement` at module scope,
 * so `HTMLElement`, `document`, and `window` must exist globally before the
 * module is evaluated — same mocking pattern as `template-element.test.ts`.
 */
Object.defineProperty(globalThis, 'HTMLElement', {
  value: class HTMLElement {
    tagName = '';
    isConnected = false;
    childNodes: unknown[] = [];
    shadowRoot = null;
    _attrs: Record<string, string> = {};

    hasAttribute(name: string): boolean {
      return Object.prototype.hasOwnProperty.call(this._attrs, name);
    }

    getAttribute(name: string): string | null {
      return Object.prototype.hasOwnProperty.call(this._attrs, name) ? this._attrs[name] : null;
    }

    setAttribute(name: string, value: string): void {
      this._attrs[name] = String(value);
    }

    removeAttribute(name: string): void {
      delete this._attrs[name];
    }
  },
  configurable: true,
});

Object.defineProperty(globalThis, 'document', {
  value: {
    readyState: 'complete',
    querySelector() {
      return null;
    },
  },
  configurable: true,
});

Object.defineProperty(globalThis, 'window', {
  value: {
    __webui: { templates: {} },
    dispatchEvent(): boolean {
      return true;
    },
  },
  configurable: true,
});

const { WebUIElement } = await import('./element.js');
const {
  registerVisibleHydrationCoordinator,
  __resetLazyHydrationContractForTests,
} = await import('./lazy-hydration.js');
type LazyHydrationTarget = import('./lazy-hydration.js').LazyHydrationTarget;

/** A no-op coordinator implementation, overridable per test. */
function fakeCoordinator(
  overrides: Partial<{
    supportsVisibleHydration(): boolean;
    observe(target: LazyHydrationTarget): void;
    observeStreamed(target: LazyHydrationTarget, state: Record<string, unknown> | undefined): void;
    disconnect(target: LazyHydrationTarget): void;
    isStreamedActivation(target: LazyHydrationTarget): boolean;
  }> = {},
) {
  return {
    supportsVisibleHydration: () => true,
    observe: () => {},
    observeStreamed: () => {},
    disconnect: () => {},
    isStreamedActivation: () => false,
    ...overrides,
  };
}

/** Bypass the `protected` modifier the same way `template-element.test.ts` does. */
function shouldDefer(el: object): boolean {
  return (el as unknown as { $shouldDeferSSRHydration(): boolean }).$shouldDeferSSRHydration();
}

function withConsoleWarn<T>(run: (calls: unknown[][]) => T): T {
  const previousWarn = console.warn;
  const calls: unknown[][] = [];
  console.warn = (...args: unknown[]) => {
    calls.push(args);
  };
  try {
    return run(calls);
  } finally {
    console.warn = previousWarn;
  }
}

describe('WebUIElement.hydration — eager default', () => {
  test('rejects unsupported hydration strategies at type-check time', () => {
    // @ts-expect-error "visibel" is not a supported hydration strategy.
    class InvalidItem extends WebUIElement {
      static override readonly hydration = 'visibel';
    }
    assert.equal(InvalidItem.hydration, 'visibel');
  });

  test('an ordinary component never defers, even with a coordinator installed', () => {
    __resetLazyHydrationContractForTests();
    registerVisibleHydrationCoordinator(fakeCoordinator());

    class EagerItem extends WebUIElement {}
    const el = new EagerItem();
    assert.equal(shouldDefer(el), false);
  });
});

describe('WebUIElement.hydration — "visible" opt-in', () => {
  test('defers once the optional coordinator is installed and supported', () => {
    __resetLazyHydrationContractForTests();
    registerVisibleHydrationCoordinator(fakeCoordinator());

    class VisibleItem extends WebUIElement {
      static override readonly hydration = 'visible';
    }
    const el = new VisibleItem();
    assert.equal(shouldDefer(el), true);
  });

  test('falls back to eager, without warning, when the coordinator reports no browser support', () => {
    __resetLazyHydrationContractForTests();
    registerVisibleHydrationCoordinator(
      fakeCoordinator({ supportsVisibleHydration: () => false }),
    );

    class VisibleItem extends WebUIElement {
      static override readonly hydration = 'visible';
    }

    withConsoleWarn((calls) => {
      const el = new VisibleItem();
      assert.equal(shouldDefer(el), false);
      assert.equal(calls.length, 0, 'missing IntersectionObserver support is an expected fallback, not a misconfiguration');
    });
  });

  test('falls back to eager and warns once when the optional entry was never installed', () => {
    __resetLazyHydrationContractForTests();

    class VisibleItem extends WebUIElement {
      static override readonly hydration = 'visible';
    }

    withConsoleWarn((calls) => {
      const first = new VisibleItem();
      assert.equal(shouldDefer(first), false);
      assert.equal(calls.length, 1, 'a missing optional entry warns in development');
      assert.match(String(calls[0][0]), /visible-hydration\.js/);

      const second = new VisibleItem();
      assert.equal(shouldDefer(second), false);
      assert.equal(calls.length, 1, 'the missing-entry warning fires at most once per session');
    });
  });
});

describe('WebUIElement — w-hydrate="eager" SSR escape hatch', () => {
  test('forces synchronous hydration for one instance, without touching the coordinator', () => {
    __resetLazyHydrationContractForTests();
    registerVisibleHydrationCoordinator(
      fakeCoordinator({
        observe: () => {
          throw new Error('must not observe an eager-escaped instance');
        },
      }),
    );

    class VisibleItem extends WebUIElement {
      static override readonly hydration = 'visible';
    }
    const el = new VisibleItem();
    (el as unknown as { setAttribute(n: string, v: string): void }).setAttribute('w-hydrate', 'eager');
    assert.equal(shouldDefer(el), false);
  });

  test('does not warn about a missing optional entry — the override makes eager intentional', () => {
    __resetLazyHydrationContractForTests();

    class VisibleItem extends WebUIElement {
      static override readonly hydration = 'visible';
    }
    const el = new VisibleItem();
    (el as unknown as { setAttribute(n: string, v: string): void }).setAttribute('w-hydrate', 'eager');

    withConsoleWarn((calls) => {
      assert.equal(shouldDefer(el), false);
      assert.equal(calls.length, 0);
    });
  });

  test('is ignored for any value other than the exact string "eager"', () => {
    __resetLazyHydrationContractForTests();
    registerVisibleHydrationCoordinator(fakeCoordinator());

    class VisibleItem extends WebUIElement {
      static override readonly hydration = 'visible';
    }
    const el = new VisibleItem();
    (el as unknown as { setAttribute(n: string, v: string): void }).setAttribute('w-hydrate', 'Eager');
    assert.equal(shouldDefer(el), true, 'a near-miss value must not silently opt an instance out');
  });

  test('ordinary eager components never read the w-hydrate attribute', () => {
    __resetLazyHydrationContractForTests();

    class EagerItem extends WebUIElement {}
    const el = new EagerItem();
    let read = false;
    (el as unknown as { getAttribute(n: string): string | null }).getAttribute = (name: string) => {
      if (name === 'w-hydrate') read = true;
      return null;
    };
    assert.equal(shouldDefer(el), false);
    assert.equal(read, false, 'the static hydration check must short-circuit before any attribute lookup');
  });
});
