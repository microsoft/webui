// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import { randomUUID } from 'node:crypto';
import { gzipSync } from 'node:zlib';
import type { StreamingFixtureAssets } from '@microsoft/webui-test-support/fixture-streaming';
import type { TemplateMeta } from '../../../src/template-types.js';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    window.__streamingCompletions = 0;
    window.addEventListener('webui:hydration-complete', () => {
      window.__streamingCompletions = (window.__streamingCompletions ?? 0) + 1;
    });
  });
});

for (const trustedTypes of [false, true]) {
  test(`processes streamed checkpoints before the application loads${trustedTypes ? ' with Trusted Types enforced' : ''}`, async ({
    page, request,
  }) => {
    const assets: StreamingFixtureAssets = await (await request.get('/streaming-bootstrap/assets')).json();
    const appGate = Promise.withResolvers<void>();
    await page.route(`**${assets.application}`, async (route) => {
      await appGate.promise;
      await route.continue();
    });
    const scripts: string[] = [];
    page.on('request', (req) => {
      if (req.resourceType() === 'script') scripts.push(new URL(req.url()).pathname);
    });
    const id = randomUUID();
    try {
      await page.goto(`/streaming-bootstrap/fixture.html?id=${id}${trustedTypes ? '&trusted=1' : ''}`, { waitUntil: 'commit' });
      await page.waitForFunction(() => performance.getEntriesByName('webui:boundary:0:update').length === 1);
      await expect(page.locator('h1')).toHaveText('Server-rendered before application startup');
      await expect(page.locator('test-stream-parent button')).toHaveText('Nested: 0');
      expect(await page.evaluate(() => ({
        loading: document.readyState === 'loading',
        application: window.__streamingApplicationStarted,
        defined: !!customElements.get('test-stream-parent'),
        state: window.__webui?.state,
        completions: window.__streamingCompletions,
      }))).toEqual({
        loading: true, application: undefined, defined: false, state: undefined, completions: 0,
      });
      const early = new Set([assets.coordinator.src, ...assets.coordinator.imports]);
      expect(scripts.length).toBeGreaterThan(0);
      expect(scripts.every((src) => early.has(src))).toBe(true);
      const payloads = await Promise.all([...new Set(scripts)].map(async (src) => {
        const response = await request.get(src);
        expect(response.ok()).toBe(true);
        return response.body();
      }));
      expect(payloads.reduce((sum, body) => sum + body.length, 0)).toBeLessThanOrEqual(32 * 1024);
      expect(payloads.reduce((sum, body) => sum + gzipSync(body).length, 0)).toBeLessThanOrEqual(11 * 1024);
      await expect(page.locator('script[data-webui-boundary], webui-hydrate')).toHaveCount(0);

      await request.post(`/streaming-bootstrap/release?id=${id}`);
      await page.waitForFunction(() => performance.getEntriesByName('webui:streaming:terminal').length === 1);
      expect(await page.evaluate(() => window.__streamingCompletions)).toBe(0);
      appGate.resolve();
      await page.waitForFunction(() => window.__streamingCompletions === 1);

      await expect(page.locator('test-stream-parent button')).toHaveText('Nested: 1');
      await expect(page.locator('test-stream-parent .active')).toHaveText('Active');
      const order = await page.evaluate(() => window.__streamingActivationOrder ?? []);
      expect(order.filter((name) =>
        name === 'test-stream-parent' || name === 'test-stream-counter:Nested',
      )).toEqual(['test-stream-parent', 'test-stream-counter:Nested']);
      expect(order.filter((name) => name === 'test-stream-counter:Last')).toHaveLength(1);
      await page.locator('test-stream-parent button').click();
      await expect(page.locator('test-stream-parent button')).toHaveText('Nested: 2');
      await expect(page.locator('[data-ws], script[data-webui-boundary], webui-hydrate')).toHaveCount(0);
      expect(await page.evaluate(() => window.__webui?.state)).toBeUndefined();

      // A fresh mount needs the catalog the early bootstrap already registered.
      await page.evaluate(() => {
        const counter = document.createElement('test-stream-counter');
        counter.id = 'client-mount';
        document.body.appendChild(counter);
      });
      await expect(page.locator('#client-mount button')).toHaveCSS('color', 'rgb(17, 34, 51)');
      expect(await page.evaluate(() => window.__streamingCompletions)).toBe(1);
    } finally {
      appGate.resolve();
      await request.post(`/streaming-bootstrap/release?id=${id}`);
    }
  });
}

test('keeps definitions deferred when the application arrives before the coordinator', async ({
  page, request,
}) => {
  const assets: StreamingFixtureAssets = await (await request.get('/streaming-bootstrap/assets')).json();
  const bootstrapGate = Promise.withResolvers<void>();
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.route(`**${assets.coordinator.src}`, async (route) => {
    await bootstrapGate.promise;
    await route.continue();
  });
  const id = randomUUID();
  try {
    await page.goto(`/streaming-bootstrap/fixture.html?id=${id}&early=1&before=1`, { waitUntil: 'commit' });
    await page.waitForFunction(() => window.__streamingApplicationStarted);
    expect(await page.evaluate(() => ({
      coordinator: !!customElements.get('webui-hydrate'),
      component: !!customElements.get('test-stream-counter'),
      complete: window.__streamingCompletions,
    }))).toEqual({ coordinator: false, component: false, complete: 0 });
    bootstrapGate.resolve();
    await page.waitForFunction(() => !!customElements.get('webui-hydrate'));

    await request.post(`/streaming-bootstrap/release?id=${id}`);
    await page.waitForFunction(() => window.__streamingCompletions === 1);
    expect(await page.evaluate(() => {
      const ctor = customElements.get('test-stream-counter') as
        (CustomElementConstructor & { observedAttributes: string[] }) | undefined;
      return ctor?.observedAttributes;
    })).toContain('label');
    await expect(page.locator('test-stream-parent button')).toHaveText('Nested: 1');
    expect(errors).toEqual([]);
  } finally {
    bootstrapGate.resolve();
    await request.post(`/streaming-bootstrap/release?id=${id}`);
  }
});

test('loads compiler-owned host support on demand without loading application code', async ({ page, request }) => {
  const assets: StreamingFixtureAssets = await (await request.get('/streaming-bootstrap/assets')).json();
  const scripts: string[] = [];
  page.on('request', (req) => {
    if (req.resourceType() === 'script') scripts.push(new URL(req.url()).pathname);
  });
  const id = randomUUID();
  try {
    await page.goto(`/streaming-bootstrap/fixture.html?id=${id}&static=1`, { waitUntil: 'commit' });
    await page.waitForFunction(() =>
      !!customElements.get('test-stream-static') &&
      !document.querySelector('test-stream-static')?.hasAttribute('data-ws'),
    );
    await expect(page.locator('test-stream-static p')).toHaveText('Static: Ready');
    expect(scripts).toContain(assets.templateHostRuntime);
    expect(scripts).not.toContain(assets.application);
    expect(await page.evaluate(() => window.__streamingApplicationStarted)).toBeUndefined();
    await request.post(`/streaming-bootstrap/release?id=${id}`);
    await page.waitForFunction(() => window.__streamingCompletions === 1);
    await expect(page.locator('[data-ws], script[data-webui-boundary], webui-hydrate')).toHaveCount(0);
  } finally {
    await request.post(`/streaming-bootstrap/release?id=${id}`);
  }
});

test('fails closed and releases pending roots when the demanded host runtime cannot load', async ({
  page, request,
}) => {
  const assets: StreamingFixtureAssets = await (await request.get('/streaming-bootstrap/assets')).json();
  const errors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  await page.route(`**${assets.templateHostRuntime}`, (route) => route.abort('failed'));
  const id = randomUUID();
  try {
    await page.goto(`/streaming-bootstrap/fixture.html?id=${id}&static=1`, { waitUntil: 'commit' });
    await expect.poll(() => errors.some((error) => error.includes('failed to load template-host runtime'))).toBe(true);
    await request.post(`/streaming-bootstrap/release?id=${id}`);
    await page.waitForLoadState('load');
    expect(await page.evaluate(() => window.__streamingCompletions)).toBe(0);
    await expect(page.locator('[data-ws], script[data-webui-boundary], webui-hydrate')).toHaveCount(0);
  } finally {
    await request.post(`/streaming-bootstrap/release?id=${id}`);
  }
});

test('joins navigation readiness for scriptless templates before any streamed host needs the runtime', async ({
  page, request,
}) => {
  const id = randomUUID();
  try {
    await page.goto(`/streaming-bootstrap/fixture.html?id=${id}&before=1`, { waitUntil: 'commit' });
    await page.waitForFunction(() => !!customElements.get('webui-hydrate'));
    const payload: {
      templates: Record<string, TemplateMeta>;
      componentStyles: unknown;
    } = await (await request.get('/streaming-bootstrap/static-templates')).json();
    const defined = await page.evaluate(async (data) => {
      const waits: PromiseLike<unknown>[] = [];
      window.dispatchEvent(new CustomEvent('webui:templates-registered', {
        detail: {
          ...data,
          waitUntil: (promise: PromiseLike<unknown>): void => { waits.push(promise); },
        },
      }));
      await Promise.all(waits);
      return !!customElements.get('test-stream-static');
    }, payload);
    expect(defined).toBe(true);
    expect(await page.evaluate(() => window.__streamingApplicationStarted)).toBeUndefined();
    await page.evaluate(() => {
      const host = document.createElement('test-stream-static') as HTMLElement & {
        setState(state: Record<string, unknown>): void;
      };
      document.body.appendChild(host);
      host.setState({ label: 'Navigation' });
    });
    await expect(page.locator('test-stream-static p')).toHaveText('Static: Navigation');
    await request.post(`/streaming-bootstrap/release?id=${id}`);
    await page.waitForFunction(() => window.__streamingCompletions === 1);
  } finally {
    await request.post(`/streaming-bootstrap/release?id=${id}`);
  }
});

test('keeps ordinary rendering and hydration independent of streaming mode', async ({ page }) => {
  await page.goto('/streaming-bootstrap/fixture.html?ordinary=1');
  await page.waitForFunction(() => window.__streamingCompletions === 1);
  expect(await page.evaluate(() => customElements.get('webui-hydrate'))).toBeUndefined();
  await expect(page.locator('test-stream-parent button')).toHaveText('Nested: 0');
  await page.locator('test-stream-parent button').click();
  await expect(page.locator('test-stream-parent button')).toHaveText('Nested: 1');
  await expect(page.locator('meta[name="webui-streaming"], [data-ws], webui-hydrate')).toHaveCount(0);
});
