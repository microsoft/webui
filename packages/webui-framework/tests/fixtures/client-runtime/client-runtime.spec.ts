// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

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

type TestRuntimeLife = HTMLElement & {
  count: number;
  hydratedCalls: number;
  attributeChanges: string[];
  propertyCalls: { oldValue: unknown; value: number; connected: boolean }[];
};
