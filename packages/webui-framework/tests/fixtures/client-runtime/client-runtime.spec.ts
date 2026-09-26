// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import type { TestRuntimeEffects } from './element.js';

test('hydratedCallback runs once for ordinary SSR, client mount, and reconnect', async ({ page }) => {
  await page.goto('/client-runtime/ordinary.html');

  const calls = await page.evaluate(() => {
    if (customElements.get('test-runtime-immediate') !== window.TestRuntimeImmediate) {
      throw new Error('ordinary definition was unexpectedly delayed');
    }
    const ssr = document.querySelector('test-runtime-life') as TestRuntimeLife;
    const client = document.createElement('test-runtime-life') as TestRuntimeLife;
    document.body.appendChild(client);
    ssr.remove();
    document.body.appendChild(ssr);
    client.remove();
    document.body.appendChild(client);
    return {
      ssr: ssr.hydratedCalls,
      client: client.hydratedCalls,
    };
  });

  expect(calls).toEqual({ ssr: 1, client: 1 });
});

test('observable callbacks reconcile the final initial value and run live synchronously', async ({ page }) => {
  await page.goto('/client-runtime/ordinary.html');
  const result = await page.evaluate(() => {
    const el = document.createElement('test-runtime-life') as TestRuntimeLife;
    el.count = 2;
    const beforeConnect = el.propertyCalls.length;
    document.body.appendChild(el);
    const initial = el.propertyCalls.slice();
    el.count = 3;
    const live = el.propertyCalls.slice();
    el.remove();
    el.count = 4;
    el.count = 5;
    const detached = el.propertyCalls.length;
    document.body.appendChild(el);
    return { beforeConnect, initial, live, detached, afterReconnect: el.propertyCalls };
  });
  expect(result).toEqual({
    beforeConnect: 0,
    initial: [{ oldValue: undefined, value: 2, connected: true }],
    live: [
      { oldValue: undefined, value: 2, connected: true },
      { oldValue: 2, value: 3, connected: true },
    ],
    detached: 2,
    afterReconnect: [
      { oldValue: undefined, value: 2, connected: true },
      { oldValue: 2, value: 3, connected: true },
      { oldValue: 3, value: 5, connected: true },
    ],
  });
});

test('callback errors surface from live property assignments', async ({ page }) => {
  await page.goto('/client-runtime/ordinary.html');
  const result = await page.evaluate(() => {
    const el = document.createElement('test-runtime-life') as TestRuntimeLife;
    document.body.appendChild(el);
    const proto = Object.getPrototypeOf(el) as { countChanged(old: unknown, value: number): void };
    const original = proto.countChanged;
    proto.countChanged = () => { throw new Error('reactive effect failed'); };
    try {
      try {
        el.count = 7;
        return { error: null, value: el.count };
      } catch (error) {
        return { error: (error as Error).message, value: el.count };
      }
    } finally {
      proto.countChanged = original;
    }
  });
  expect(result).toEqual({ error: 'reactive effect failed', value: 7 });
});

for (const teardown of [false, true]) {
  test(`resumes live callbacks after ${teardown ? 'teardown' : 'a move'} without detached writes`, async ({ page }) => {
    await page.goto('/client-runtime/ordinary.html');
    const result = await page.evaluate(async (teardown) => {
      const el = document.createElement('test-runtime-life') as TestRuntimeLife;
      document.body.appendChild(el);
      el.remove();
      if (teardown) await new Promise<void>(resolve => queueMicrotask(resolve));
      document.body.appendChild(el);
      const afterReconnect = el.propertyCalls.length;
      el.count = 7;
      el.$flushUpdates();
      el.$flushUpdates();
      return { afterReconnect, calls: el.propertyCalls, hydratedCalls: el.hydratedCalls };
    }, teardown);
    expect(result).toEqual({
      afterReconnect: 1,
      calls: [
        { oldValue: undefined, value: 0, connected: true },
        { oldValue: 0, value: 7, connected: true },
      ],
      hydratedCalls: 1,
    });
  });
}

test('a throwing hydratedCallback is latched before author code runs', async ({ page }) => {
  await page.goto('/client-runtime/ordinary.html');

  const calls = await page.evaluate(() => {
    const el = new window.TestRuntimeThrow();
    try {
      el.connectedCallback();
    } catch {
      // Expected author exception.
    }
    el.connectedCallback();
    return el.hydratedCalls;
  });

  expect(calls).toBe(1);
});

test('early streaming definition waits for metadata and native observedAttributes', async ({ page }) => {
  await page.goto('/client-runtime/streaming.html');

  expect(await page.evaluate(() => customElements.get('test-runtime-life') === undefined)).toBe(true);

  const result = await page.evaluate(() => {
    window.registerClientRuntimeTemplates();
    const ctor = customElements.get('test-runtime-life');
    const el = document.createElement('test-runtime-life') as TestRuntimeLife;
    document.body.appendChild(el);
    el.setAttribute('label', 'ready');
    return {
      authoredWon: ctor === window.TestRuntimeLife,
      observed: (ctor as CustomElementConstructor & {
        observedAttributes?: readonly string[];
      } | undefined)?.observedAttributes,
      changes: el.attributeChanges,
    };
  });

  expect(result).toEqual({
    authoredWon: true,
    observed: ['label'],
    changes: ['label'],
  });
});

test('ordinary Router definition waits for metadata and renders template-only attributes', async ({ page }) => {
  await page.goto('/client-runtime/router.html');

  expect(await page.evaluate(() => customElements.get('test-runtime-life') === undefined)).toBe(true);

  const result = await page.evaluate(() => {
    window.registerClientRuntimeTemplates();
    const ctor = customElements.get('test-runtime-life');
    const el = document.createElement('test-runtime-life') as TestRuntimeLife;
    el.setAttribute('label', 'ready');
    document.body.appendChild(el);
    return {
      authoredWon: ctor === window.TestRuntimeLife,
      observed: (ctor as CustomElementConstructor & {
        observedAttributes?: readonly string[];
      } | undefined)?.observedAttributes,
      changes: el.attributeChanges,
      text: (el.shadowRoot ?? el).querySelector('span')?.textContent,
    };
  });

  expect(result).toEqual({
    authoredWon: true,
    observed: ['label'],
    changes: ['label'],
    text: 'ready',
  });
});

test('streamed activation fires once, including detached late definition', async ({ page }) => {
  await page.goto('/client-runtime/streaming.html');

  const attached = await page.evaluate((activationKey) => {
    window.registerClientRuntimeTemplates();
    const el = document.createElement('test-runtime-life') as TestRuntimeLife;
    el.setAttribute('data-ws', '');
    el.innerHTML = '<span></span>';
    document.body.appendChild(el);
    const before = el.hydratedCalls;
    const outcome = (el as unknown as Record<symbol, () => number>)[
      Symbol.for(activationKey)
    ]();
    el.remove();
    document.body.appendChild(el);
    return { before, outcome, after: el.hydratedCalls };
  }, 'microsoft.webui.boundaryActivate');

  expect(attached).toEqual({ before: 0, outcome: 1, after: 1 });

  await page.goto('/client-runtime/streaming.html');
  const detached = await page.evaluate((activationKey) => {
    const el = document.createElement('test-runtime-life') as TestRuntimeLife;
    el.setAttribute('data-ws', '');
    el.innerHTML = '<span></span>';
    document.body.appendChild(el);
    el.remove();
    window.registerClientRuntimeTemplates();
    customElements.upgrade(el);
    const outcome = (el as unknown as Record<symbol, () => number>)[
      Symbol.for(activationKey)
    ]();
    return {
      connected: el.isConnected,
      outcome,
      calls: el.hydratedCalls,
      propertyCalls: el.propertyCalls.length,
      afterConnect: (() => {
        document.body.appendChild(el);
        return el.propertyCalls;
      })(),
    };
  }, 'microsoft.webui.boundaryActivate');

  expect(detached).toEqual({
    connected: false, outcome: 1, calls: 1, propertyCalls: 0,
    afterConnect: [{ oldValue: undefined, value: 0, connected: true }],
  });
});

test.describe('authored callback recovery', () => {
  test('latest detached-upgrade writes win for observable and attr properties', async ({ page }) => {
    await page.goto('/client-runtime/streaming.html');
    const result = await page.evaluate(() => {
      const el = document.createElement('test-runtime-effects') as TestRuntimeEffects;
      el.a = 1;
      el.label = 'Before upgrade';
      window.registerClientRuntimeTemplates();
      customElements.upgrade(el);
      el.a = 2;
      el.label = 'After upgrade';
      const beforeConnect = el.calls.slice();
      document.body.appendChild(el);
      return {
        beforeConnect,
        a: el.a,
        label: el.label,
        attribute: el.getAttribute('label'),
        calls: el.calls,
      };
    });
    expect(result).toEqual({
      beforeConnect: [],
      a: 2,
      label: 'After upgrade',
      attribute: 'After upgrade',
      calls: [
        { name: 'a', oldValue: undefined, value: 2, connected: true },
        { name: 'b', oldValue: undefined, value: 0, connected: true },
        { name: 'hydrated', connected: true },
      ],
    });
  });

  for (const connected of [false, true]) {
    test(`preserves pre-upgrade property precedence over initial attributes during ${connected ? 'connected' : 'detached'} upgrade`, async ({ page }) => {
      await page.goto('/client-runtime/streaming.html');
      const result = await page.evaluate((connected) => {
        const el = document.createElement('test-runtime-effects') as TestRuntimeEffects;
        el.setAttribute('label', 'Initial markup');
        el.label = 'Imperative before upgrade';
        if (connected) document.body.appendChild(el);
        window.registerClientRuntimeTemplates();
        if (!connected) {
          customElements.upgrade(el);
          document.body.appendChild(el);
        }
        const initial = { label: el.label, attribute: el.getAttribute('label') };
        el.setAttribute('label', 'Later attribute change');
        return {
          initial,
          afterAttributeChange: { label: el.label, attribute: el.getAttribute('label') },
        };
      }, connected);
      expect(result).toEqual({
        initial: {
          label: 'Imperative before upgrade', attribute: 'Imperative before upgrade',
        },
        afterAttributeChange: {
          label: 'Later attribute change', attribute: 'Later attribute change',
        },
      });
    });
  }

  test('pauses initial property callbacks when a callback disconnects the host', async ({ page }) => {
    await page.goto('/client-runtime/ordinary.html');
    const result = await page.evaluate(async () => {
      const el = document.createElement('test-runtime-effects') as TestRuntimeEffects;
      el.onAChange = () => {
        el.onAChange = undefined;
        el.remove();
      };
      document.body.appendChild(el);
      el.b = 7;
      await new Promise<void>(resolve => queueMicrotask(resolve));
      el.b = 8;
      el.$flushUpdates();
      const connected = el.isConnected;
      const whileDetached = el.calls.filter(call => call.name !== 'hydrated');
      document.body.appendChild(el);
      el.$flushUpdates();
      el.$flushUpdates();
      return {
        connected,
        whileDetached,
        afterReconnect: el.calls.filter(call => call.name !== 'hydrated'),
      };
    });
    expect(result).toEqual({
      connected: false,
      whileDetached: [
        { name: 'a', oldValue: undefined, value: 0, connected: true },
      ],
      afterReconnect: [
        { name: 'a', oldValue: undefined, value: 0, connected: true },
        { name: 'b', oldValue: undefined, value: 8, connected: true },
      ],
    });
  });

  test('pauses reconnect property callbacks when a callback disconnects the host', async ({ page }) => {
    await page.goto('/client-runtime/ordinary.html');
    const result = await page.evaluate(async () => {
      const el = document.createElement('test-runtime-effects') as TestRuntimeEffects;
      document.body.appendChild(el);
      const initialCount = el.calls.length;
      el.remove();
      el.a = 1;
      el.b = 1;
      el.onAChange = () => {
        el.onAChange = undefined;
        el.remove();
      };
      document.body.appendChild(el);
      el.b = 2;
      await new Promise<void>(resolve => queueMicrotask(resolve));
      el.$flushUpdates();
      const connected = el.isConnected;
      const whileDetached = el.calls.slice(initialCount);
      document.body.appendChild(el);
      el.$flushUpdates();
      el.$flushUpdates();
      return { connected, whileDetached, afterReconnect: el.calls.slice(initialCount) };
    });
    expect(result).toEqual({
      connected: false,
      whileDetached: [
        { name: 'a', oldValue: 0, value: 1, connected: true },
      ],
      afterReconnect: [
        { name: 'a', oldValue: 0, value: 1, connected: true },
        { name: 'b', oldValue: 0, value: 2, connected: true },
      ],
    });
  });

  test('defers a child property write from its native parent disconnectedCallback', async ({ page }) => {
    await page.goto('/client-runtime/ordinary.html');
    const result = await page.evaluate(async () => {
      const child = document.createElement('test-runtime-life') as TestRuntimeLife;
      let connectedDuringParentCallback: boolean | undefined;
      class NativeParent extends HTMLElement {
        disconnectedCallback(): void {
          connectedDuringParentCallback = child.isConnected;
          child.count = 7;
        }
      }
      customElements.define('test-runtime-native-parent', NativeParent);
      const parent = document.createElement('test-runtime-native-parent');
      parent.appendChild(child);
      document.body.appendChild(parent);
      const initialCount = child.propertyCalls.length;
      parent.remove();
      const whileDetached = child.propertyCalls.slice(initialCount);
      await new Promise<void>(resolve => queueMicrotask(resolve));
      document.body.appendChild(parent);
      child.$flushUpdates();
      return {
        connectedDuringParentCallback,
        whileDetached,
        afterReconnect: child.propertyCalls.slice(initialCount),
        hydratedCalls: child.hydratedCalls,
      };
    });
    expect(result).toEqual({
      connectedDuringParentCallback: false,
      whileDetached: [],
      afterReconnect: [{ oldValue: 0, value: 7, connected: true }],
      hydratedCalls: 1,
    });
  });

  test('flush completes hydration once after the sole initial callback throws', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto('/client-runtime/ordinary.html');
    const result = await page.evaluate(async () => {
      const el = document.createElement('test-runtime-life') as InstanceType<typeof window.TestRuntimeLife>;
      const original = el.countChanged;
      el.countChanged = (oldValue, value) => {
        original.call(el, oldValue, value);
        throw new Error('expected sole initial callback failure');
      };
      document.body.appendChild(el);
      const beforeFlush = { calls: el.propertyCalls.slice(), hydratedCalls: el.hydratedCalls };
      el.countChanged = original;
      el.$flushUpdates();
      const afterFlush = { calls: el.propertyCalls.slice(), hydratedCalls: el.hydratedCalls };
      el.$flushUpdates();
      await new Promise<void>(resolve => queueMicrotask(resolve));
      el.remove();
      document.body.appendChild(el);
      return {
        beforeFlush,
        afterFlush,
        afterReconnect: { calls: el.propertyCalls, hydratedCalls: el.hydratedCalls },
      };
    });
    await expect.poll(() => errors).toEqual(['expected sole initial callback failure']);
    const calls = [{ oldValue: undefined, value: 0, connected: true }];
    expect(result).toEqual({
      beforeFlush: { calls, hydratedCalls: 0 },
      afterFlush: { calls, hydratedCalls: 1 },
      afterReconnect: { calls, hydratedCalls: 1 },
    });
  });

  test('flush completes hydration once after the last initial callback throws', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto('/client-runtime/ordinary.html');
    const result = await page.evaluate(async () => {
      const el = document.createElement('test-runtime-effects') as TestRuntimeEffects;
      const original = el.bChanged;
      el.bChanged = (oldValue, value) => {
        original.call(el, oldValue, value);
        throw new Error('expected last initial callback failure');
      };
      document.body.appendChild(el);
      const beforeFlush = el.calls.slice();
      el.bChanged = original;
      el.$flushUpdates();
      const afterFlush = el.calls.slice();
      el.$flushUpdates();
      await new Promise<void>(resolve => queueMicrotask(resolve));
      el.remove();
      document.body.appendChild(el);
      return { beforeFlush, afterFlush, afterReconnect: el.calls };
    });
    await expect.poll(() => errors).toEqual(['expected last initial callback failure']);
    const initial = [
      { name: 'a', oldValue: undefined, value: 0, connected: true },
      { name: 'b', oldValue: undefined, value: 0, connected: true },
    ];
    const recovered = [...initial, { name: 'hydrated', connected: true }];
    expect(result).toEqual({ beforeFlush: initial, afterFlush: recovered, afterReconnect: recovered });
  });

  test('recovers unentered initial work before entering hydratedCallback exactly once', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto('/client-runtime/ordinary.html');
    const result = await page.evaluate(async () => {
      const el = document.createElement('test-runtime-effects') as TestRuntimeEffects;
      el.onAChange = () => {
        el.onAChange = undefined;
        throw new Error('expected initial aChanged failure');
      };
      document.body.appendChild(el);
      const beforeRecovery = el.calls.slice();
      el.remove();
      await new Promise<void>(resolve => queueMicrotask(resolve));
      document.body.appendChild(el);
      const afterRecovery = el.calls.slice();
      el.$flushUpdates();
      el.$flushUpdates();
      el.remove();
      await new Promise<void>(resolve => queueMicrotask(resolve));
      document.body.appendChild(el);
      return { beforeRecovery, afterRecovery, afterAnotherReconnect: el.calls };
    });
    await expect.poll(() => errors).toEqual(['expected initial aChanged failure']);
    const recovered = [
      { name: 'a', oldValue: undefined, value: 0, connected: true },
      { name: 'b', oldValue: undefined, value: 0, connected: true },
      { name: 'hydrated', connected: true },
    ];
    expect(result).toEqual({
      beforeRecovery: [recovered[0]],
      afterRecovery: recovered,
      afterAnotherReconnect: recovered,
    });
  });

  for (const teardown of [false, true]) {
    test(`coalesces a reentrant pending property once after ${teardown ? 'teardown' : 'a move'}`, async ({ page }) => {
      await page.goto('/client-runtime/ordinary.html');
      const result = await page.evaluate(async (teardown) => {
        const el = document.createElement('test-runtime-effects') as TestRuntimeEffects;
        document.body.appendChild(el);
        const initialCount = el.calls.length;
        el.remove();
        el.a = 1;
        el.b = 1;
        if (teardown) await new Promise<void>(resolve => queueMicrotask(resolve));
        el.onAChange = () => { el.b = 2; };
        document.body.appendChild(el);
        const reconciled = el.calls.slice(initialCount);
        el.b = 3;
        el.b = 4;
        const afterLiveWrites = el.calls.slice(initialCount);
        el.$flushUpdates();
        el.$flushUpdates();
        await new Promise<void>(resolve => queueMicrotask(resolve));
        return { reconciled, afterLiveWrites, afterFlush: el.calls.slice(initialCount) };
      }, teardown);
      const reconciled = [
        { name: 'a', oldValue: 0, value: 1, connected: true },
        { name: 'b', oldValue: 0, value: 2, connected: true },
      ];
      const afterLiveWrites = [
        ...reconciled,
        { name: 'b', oldValue: 2, value: 3, connected: true },
        { name: 'b', oldValue: 3, value: 4, connected: true },
      ];
      expect(result).toEqual({ reconciled, afterLiveWrites, afterFlush: afterLiveWrites });
    });
  }
});

type TestRuntimeLife = HTMLElement & {
  $flushUpdates(): void;
  count: number;
  hydratedCalls: number;
  attributeChanges: string[];
  propertyCalls: { oldValue: unknown; value: number; connected: boolean }[];
};
