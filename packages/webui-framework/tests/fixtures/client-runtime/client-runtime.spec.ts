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
    };
  }, 'microsoft.webui.boundaryActivate');

  expect(detached).toEqual({ connected: false, outcome: 1, calls: 1 });
});

type TestRuntimeLife = HTMLElement & {
  hydratedCalls: number;
  attributeChanges: string[];
};
