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
  registerLazyHydrationCoordinator,
  __resetLazyHydrationContractForTests,
} = await import('./lazy-hydration-contract.js');
type LazyHydrationTarget = import('./lazy-hydration-contract.js').LazyHydrationTarget;
type LazyHydrationMode = import('./lazy-hydration-contract.js').LazyHydrationMode;
type TemplateMeta = import('./template.js').TemplateMeta;

/** A no-op coordinator implementation, overridable per test. */
function fakeCoordinator(
  overrides: Partial<{
    supportsLazyHydration(): boolean;
    observe(target: LazyHydrationTarget, mode: LazyHydrationMode): void;
    observeStreamed(
      target: LazyHydrationTarget,
      state: Record<string, unknown> | undefined,
      mode: LazyHydrationMode,
    ): void;
    disconnect(target: LazyHydrationTarget): void;
    isStreamedActivation(target: LazyHydrationTarget): boolean;
  }> = {},
) {
  return {
    supportsLazyHydration: () => true,
    observe: () => {},
    observeStreamed: () => {},
    disconnect: () => {},
    isStreamedActivation: () => false,
    ...overrides,
  };
}

/** Bypass the `protected` modifier the same way `template-element.test.ts` does. */
function shouldDefer(el: object, wp?: 1 | 2 | 3 | 4): boolean {
  return (
    el as unknown as {
      $shouldDeferSSRHydration(meta: TemplateMeta): boolean;
    }
  ).$shouldDeferSSRHydration({ h: '', wp });
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

describe('WebUIElement — eager default', () => {
  test('an ordinary component never defers, even with a coordinator installed', () => {
    __resetLazyHydrationContractForTests();
    registerLazyHydrationCoordinator(fakeCoordinator());

    class EagerItem extends WebUIElement {}
    const el = new EagerItem();
    assert.equal(shouldDefer(el), false);
  });

  test('an ordinary component skips coordinator disconnect bookkeeping', () => {
    __resetLazyHydrationContractForTests();
    let disconnectCalls = 0;
    registerLazyHydrationCoordinator(
      fakeCoordinator({
        disconnect: () => {
          disconnectCalls++;
        },
      }),
    );

    class EagerItem extends WebUIElement {}
    const el = new EagerItem();
    el.disconnectedCallback();
    assert.equal(disconnectCalls, 0);
  });
});

describe('WebUIElement — compiler-owned work policy', () => {
  test('defers lazy hydration once the optional coordinator is installed and supported', () => {
    __resetLazyHydrationContractForTests();
    registerLazyHydrationCoordinator(fakeCoordinator());

    class LazyItem extends WebUIElement {}
    const el = new LazyItem();
    assert.equal(shouldDefer(el, 1), true);
  });

  test('defers the combined offscreen policy through the same coordinator', () => {
    __resetLazyHydrationContractForTests();
    registerLazyHydrationCoordinator(fakeCoordinator());

    class OffscreenItem extends WebUIElement {}
    const el = new OffscreenItem();
    assert.equal(shouldDefer(el, 2), true);
  });

  test('interaction policy hydrates eagerly once its deferred module loads', () => {
    __resetLazyHydrationContractForTests();

    class InteractionItem extends WebUIElement {}
    const el = new InteractionItem();
    assert.equal(shouldDefer(el, 3), false);
  });

  test('combined render and interaction policy hydrates eagerly after loading', () => {
    __resetLazyHydrationContractForTests();
    registerLazyHydrationCoordinator(fakeCoordinator());

    class InteractionItem extends WebUIElement {}
    const el = new InteractionItem();
    assert.equal(shouldDefer(el, 4), false);
  });

  test('falls back to eager, without warning, when the coordinator reports no browser support', () => {
    __resetLazyHydrationContractForTests();
    registerLazyHydrationCoordinator(
      fakeCoordinator({ supportsLazyHydration: () => false }),
    );

    class LazyItem extends WebUIElement {}

    withConsoleWarn((calls) => {
      const el = new LazyItem();
      assert.equal(shouldDefer(el, 1), false);
      assert.equal(calls.length, 0, 'missing IntersectionObserver support is an expected fallback, not a misconfiguration');
    });
  });

  test('falls back to eager and warns once when the optional entry was never installed', () => {
    __resetLazyHydrationContractForTests();

    class LazyItem extends WebUIElement {}

    withConsoleWarn((calls) => {
      const first = new LazyItem();
      assert.equal(shouldDefer(first, 1), false);
      assert.equal(calls.length, 1, 'a missing optional entry warns in development');
      assert.match(String(calls[0][0]), /lazy-hydration\.js/);

      const second = new LazyItem();
      assert.equal(shouldDefer(second, 1), false);
      assert.equal(calls.length, 1, 'the missing-entry warning fires at most once per session');
    });
  });
});

describe('WebUIElement — w-hydrate="eager" SSR escape hatch', () => {
  test('forces synchronous hydration for one instance, without touching the coordinator', () => {
    __resetLazyHydrationContractForTests();
    registerLazyHydrationCoordinator(
      fakeCoordinator({
        observe: () => {
          throw new Error('must not observe an eager-escaped instance');
        },
      }),
    );

    class LazyItem extends WebUIElement {}
    const el = new LazyItem();
    (el as unknown as { setAttribute(n: string, v: string): void }).setAttribute('w-hydrate', 'eager');
    assert.equal(shouldDefer(el, 1), false);
  });

  test('does not warn about a missing optional entry — the override makes eager intentional', () => {
    __resetLazyHydrationContractForTests();

    class LazyItem extends WebUIElement {}
    const el = new LazyItem();
    (el as unknown as { setAttribute(n: string, v: string): void }).setAttribute('w-hydrate', 'eager');

    withConsoleWarn((calls) => {
      assert.equal(shouldDefer(el, 1), false);
      assert.equal(calls.length, 0);
    });
  });

  test('is ignored for any value other than the exact string "eager"', () => {
    __resetLazyHydrationContractForTests();
    registerLazyHydrationCoordinator(fakeCoordinator());

    class LazyItem extends WebUIElement {}
    const el = new LazyItem();
    (el as unknown as { setAttribute(n: string, v: string): void }).setAttribute('w-hydrate', 'Eager');
    assert.equal(shouldDefer(el, 1), true, 'a near-miss value must not silently opt an instance out');
  });

  test('w-render="eager" disables the complete offscreen policy', () => {
    __resetLazyHydrationContractForTests();
    registerLazyHydrationCoordinator(fakeCoordinator());

    class OffscreenItem extends WebUIElement {}
    const el = new OffscreenItem();
    el.setAttribute('w-render', 'eager');
    assert.equal(shouldDefer(el, 2), false);
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
    assert.equal(read, false, 'compiler metadata must short-circuit before any attribute lookup');
  });
});
