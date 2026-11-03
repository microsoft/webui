// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';
import type { PreviewPage } from './src/preview-page.js';

interface Graph {
  application: string;
  registrationBytes: number;
  registration: string;
  independentEntryRegistration: string;
  outputs: Record<string, { bytes: number; inputs: string[] }>;
  packs: { region: string; path: string; stylesheets: string[] }[];
  routePacks: Graph['packs'];
}

function behavior(graph: Graph, name: string): string {
  const found = Object.entries(graph.outputs).find(([, output]) =>
    output.inputs.some(input => input.endsWith(`/lazy-pages/src/${name}.ts`)));
  if (!found) throw new Error(`Missing emitted ${name} chunk`);
  return found[0];
}

function pack(graph: Graph, owner: string, index: number): string {
  const found = graph.packs.find(entry => entry.region === `${owner}::lazy:@${index}`);
  if (!found) throw new Error(`Missing ${owner} region ${index}`);
  return found.path;
}

async function observe(page: Page, kind: 'manual' | 'routed' = 'manual'): Promise<{ graph: Graph; requests: string[] }> {
  // Drive best-effort idle independently of input, without wall-clock assumptions.
  await page.addInitScript(() => {
    const callbacks: IdleRequestCallback[] = [];
    window.requestIdleCallback = callback => callbacks.push(callback);
    Object.assign(window, {
      runIdle() {
        const pending = callbacks.splice(0);
        for (const callback of pending) callback({ didTimeout: false, timeRemaining: () => 50 });
      },
    });
  });
  const graph: Graph = await (await page.request.get('/lazy-pages/graph')).json();
  if (kind === 'routed') graph.packs = graph.routePacks;
  const requests: string[] = [];
  page.on('request', request => requests.push(new URL(request.url()).pathname));
  return { graph, requests };
}

async function setup(page: Page): Promise<{ graph: Graph; requests: string[] }> {
  const { graph, requests } = await observe(page);
  await page.goto('/lazy-pages/fixture.html');
  await expect(page.locator('pages-shell .first')).toBeVisible();
  expect(graph.registration).not.toContain('lazy-regions.webui');
  expect(graph.registrationBytes).toBeGreaterThan(0);
  expect(graph.registration).toContain('preview-page::lazy:');
  expect(graph.registration).toContain('unrelated-page::lazy:');
  expect(graph.independentEntryRegistration).toContain('unrelated-page::lazy:');
  expect(graph.independentEntryRegistration).not.toContain('preview-page::lazy:');
  expect(graph.independentEntryRegistration).not.toContain('help-panel');
  expect(requests).not.toContain(behavior(graph, 'preview-page'));
  expect(requests).not.toContain(behavior(graph, 'unrelated-page'));
  for (const tag of ['help-panel', 'shared-panel', 'nested-panel', 'sibling-panel', 'idle-panel', 'unrelated-panel']) {
    expect(requests).not.toContain(behavior(graph, tag));
  }
  expect(requests.filter(path => path.includes('/_webui/packs/'))).toEqual([]);
  await page.locator('pages-shell .first').click();
  await expect(page.locator('preview-page .preview')).toBeVisible();
  expect(requests).toContain(behavior(graph, 'preview-page'));
  expect(requests).not.toContain(behavior(graph, 'help-panel'));
  expect(requests).not.toContain(behavior(graph, 'unrelated-page'));
  expect(requests.filter(path => path.includes('/_webui/packs/'))).toEqual([]);
  return { graph, requests };
}

test('manual page switch: hover isolates its comparison region and overlaps pack, code and CSS', async ({ page }) => {
  const { graph, requests } = await setup(page);
  const previewPack = pack(graph, 'preview-page', 0);
  const pending = Promise.withResolvers<void>();
  await page.route(`**${previewPack}`, async route => {
    await pending.promise;
    await route.continue();
  });
  const packRequest = page.waitForRequest(`**${previewPack}`);
  const codeRequest = page.waitForRequest(`**${behavior(graph, 'help-panel')}`);
  const cssRequest = page.waitForRequest('**/lazy-pages/help-panel.css');
  await page.locator('preview-page .preview').hover();
  await Promise.all([packRequest, codeRequest, cssRequest]);
  // The held pack proves the known code/CSS do not wait for its response.
  await expect(page.locator('help-panel')).toHaveCount(0);
  expect(requests).not.toContain(pack(graph, 'preview-page', 1));
  expect(requests).not.toContain(pack(graph, 'preview-page', 2));
  expect(requests).not.toContain(pack(graph, 'preview-page', 3));
  expect(requests).not.toContain(behavior(graph, 'nested-panel'));
  expect(requests).not.toContain(behavior(graph, 'sibling-panel'));
  expect(requests).not.toContain(behavior(graph, 'unrelated-panel'));
  // Repeated activation joins the same in-flight pack.
  await page.locator('preview-page').evaluate(element => (element as PreviewPage).preview());
  expect(requests.filter(path => path === previewPack)).toHaveLength(1);
  pending.resolve();
  await expect(page.locator('help-panel p')).toHaveText('Page-owned value');
  await expect(page.locator('help-panel p')).toHaveCSS('color', 'rgb(17, 81, 139)');
  expect(await page.locator('preview-page').evaluate(element => {
    const host = element as PreviewPage;
    const child = host.shadowRoot!.querySelector('help-panel') as HTMLElement & { textNode: HTMLParagraphElement };
    return host.previewNode === host.shadowRoot!.querySelector('.preview-body')
      && child.textNode === child.shadowRoot!.querySelector('p');
  })).toBe(true);
  await page.locator('preview-page .owner-event').click();
  await expect(page.locator('preview-page .owner-event')).toHaveText('1');
  await page.locator('preview-page .nested').click();
  await expect(page.locator('nested-panel')).toHaveText('Nested only');
  expect(requests).toContain(behavior(graph, 'nested-panel'));
  expect(requests).not.toContain(behavior(graph, 'unrelated-page'));
  await page.locator('preview-page .sibling').click();
  await expect(page.locator('sibling-panel')).toHaveText('Sibling only');
  expect(requests.filter(path => path === behavior(graph, 'shared-panel'))).toHaveLength(1);
  expect(requests.filter(path => path === '/lazy-pages/help-panel.css')).toHaveLength(1);
});

test('manual page switch: selective idle and path prefetch do not mount or fetch state', async ({ page }) => {
  const { graph, requests } = await setup(page);
  const idle = pack(graph, 'preview-page', 3);
  const warmed = page.waitForRequest(`**${idle}`);
  await page.evaluate(() => (window as unknown as { runIdle(): void }).runIdle());
  await warmed;
  await page.waitForFunction(() => !!window.__webui?.templates?.['preview-page::lazy:@3']);
  await expect(page.locator('idle-panel')).toHaveCount(0);
  expect(requests).not.toContain(pack(graph, 'preview-page', 0));
  expect(requests).not.toContain(pack(graph, 'preview-page', 1));
  const preview = page.waitForRequest(`**${pack(graph, 'preview-page', 0)}`);
  const comparison = page.waitForRequest(`**${pack(graph, 'preview-page', 4)}`);
  await page.locator('preview-page .prefetch').click();
  await Promise.all([preview, comparison]);
  await page.waitForFunction(() => !!window.__webui?.templates?.['preview-page::lazy:@0']);
  await expect(page.locator('help-panel, nested-panel, sibling-panel')).toHaveCount(0);
  expect(requests).not.toContain(behavior(graph, 'nested-panel'));
  expect(requests).toContain(behavior(graph, 'sibling-panel'));
  expect(requests).not.toContain(pack(graph, 'preview-page', 2));
  expect(requests).not.toContain(behavior(graph, 'unrelated-page'));
  expect(requests.filter(path => /state|manifest/.test(path))).toEqual([]);
});

test('manual page switch: rapid close cannot mount a completed load; reactivation reuses resources', async ({ page }) => {
  const { graph, requests } = await setup(page);
  const previewPack = pack(graph, 'preview-page', 0);
  const gate = Promise.withResolvers<void>();
  await page.route(`**${previewPack}`, async route => {
    await gate.promise;
    await route.continue();
  });
  const requested = page.waitForRequest(`**${previewPack}`);
  await page.locator('preview-page .preview').hover();
  await requested;
  await page.mouse.move(0, 0);
  gate.resolve();
  await page.locator('preview-page').evaluate(async element => {
    await (element as PreviewPage).$rendered();
  });
  await expect(page.locator('help-panel')).toHaveCount(0);
  expect(await page.locator('preview-page').evaluate(element => (element as PreviewPage).previewNode)).toBeUndefined();
  await page.locator('preview-page .preview').hover();
  await expect(page.locator('help-panel p')).toHaveText('Page-owned value');
  expect(requests.filter(path => path === previewPack)).toHaveLength(1);
  expect(requests.filter(path => path === behavior(graph, 'help-panel'))).toHaveLength(1);
  // Changing page disposes the owner even when independent nested regions exist.
  await page.locator('pages-shell .second').click();
  await expect(page.locator('preview-page')).toHaveCount(0);
  await expect(page.locator('unrelated-page .unrelated')).toBeVisible();
  expect(requests).not.toContain(behavior(graph, 'unrelated-panel'));
  await page.locator('unrelated-page .unrelated').click();
  await expect(page.locator('unrelated-panel')).toHaveText('Unrelated only');
});

async function activeRoute(page: Page, component: string): Promise<void> {
  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { lazyRouteRouter: { activeComponent: string } })
      .lazyRouteRouter?.activeComponent,
  )).toBe(component);
}

function bootstrap(html: string) {
  const script = html.indexOf('id="webui-data"');
  if (script < 0) throw new Error('Routed SSR must publish bootstrap metadata.');
  const start = html.indexOf('>', script) + 1;
  const end = html.indexOf('</script>', start);
  const meta: { chain: { component: string }[]; inventory: string; buildId: string } =
    JSON.parse(html.slice(start, end));
  return {
    chain: meta.chain.map(entry => entry.component),
    inventory: meta.inventory,
    buildId: meta.buildId,
  };
}

async function navigate(page: Page, path: string): Promise<{
  requestInventory: string | undefined;
  data: { buildId: string; inventory: string; chain: { component: string }[]; templates: Record<string, unknown> };
}> {
  const response = page.waitForResponse(response =>
    new URL(response.url()).pathname === path
      && response.request().headers().accept === 'application/json',
  );
  await page.locator(`nav a[href="${path}"]`).click();
  const partial = await response;
  expect(partial.ok()).toBe(true);
  return {
    requestInventory: partial.request().headers()['x-webui-inventory'],
    data: await partial.json(),
  };
}

test('real router: SSR chain, partial navigation and hover stay isolated across collection and return', async ({ page }) => {
  const { graph, requests } = await observe(page, 'routed');
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  const document = await page.goto('/lazy-routes/');
  if (!document) throw new Error('Missing SSR document response.');
  await activeRoute(page, 'routed-home');
  await expect(page.locator('routed-shell main routed-home h2')).toHaveText('Home from SSR');
  const initial = bootstrap(await document.text());
  expect(initial.chain).toEqual(['routed-shell', 'routed-home']);
  expect(initial.inventory).toBeTruthy();
  expect(initial.buildId).toBeTruthy();
  expect(requests).not.toContain(behavior(graph, 'preview-page'));
  expect(requests).not.toContain(behavior(graph, 'unrelated-page'));
  for (const tag of ['help-panel', 'shared-panel', 'nested-panel', 'sibling-panel', 'idle-panel', 'unrelated-panel']) {
    expect(requests).not.toContain(behavior(graph, tag));
  }
  expect(requests.filter(path => path.includes('/_webui/packs/'))).toEqual([]);
  await page.locator('body').evaluate(body => { body.dataset.document = 'original'; });

  const first = await navigate(page, '/lazy-routes/preview');
  await activeRoute(page, 'preview-page');
  expect(first.requestInventory).toBe(initial.inventory);
  expect(first.data.buildId).toBe(initial.buildId);
  expect(first.data.chain.map(entry => entry.component)).toEqual(['routed-shell', 'preview-page']);
  expect(first.data.templates).toHaveProperty('preview-page');
  await expect(page.locator('preview-page .preview')).toBeVisible();
  expect(requests).toContain(behavior(graph, 'preview-page'));
  expect(requests).not.toContain(behavior(graph, 'unrelated-page'));
  expect(requests.filter(path => path.includes('/_webui/packs/'))).toEqual([]);

  const previewPack = pack(graph, 'preview-page', 0);
  const held = Promise.withResolvers<void>();
  await page.route(`**${previewPack}`, async route => {
    await held.promise;
    await route.continue();
  });
  const packRequest = page.waitForRequest(`**${previewPack}`);
  const codeRequest = page.waitForRequest(`**${behavior(graph, 'help-panel')}`);
  const cssRequest = page.waitForRequest('**/lazy-pages/help-panel.css');
  await page.locator('preview-page .preview').hover();
  await Promise.all([packRequest, codeRequest, cssRequest]);
  await expect(page.locator('help-panel')).toHaveCount(0);
  for (const index of [1, 2, 3, 4]) expect(requests).not.toContain(pack(graph, 'preview-page', index));
  for (const tag of ['nested-panel', 'sibling-panel', 'idle-panel', 'unrelated-page', 'unrelated-panel']) {
    expect(requests).not.toContain(behavior(graph, tag));
  }
  held.resolve();
  await expect(page.locator('help-panel p')).toHaveText('Page-owned value');
  await expect(page.locator('help-panel p')).toHaveCSS('color', 'rgb(17, 81, 139)');
  expect(await page.locator('preview-page').evaluate(element =>
    (element as PreviewPage).previewNode === element.shadowRoot!.querySelector('.preview-body'),
  )).toBe(true);
  await page.locator('preview-page .owner-event').click();
  await expect(page.locator('preview-page .owner-event')).toHaveText('1');

  const other = await navigate(page, '/lazy-routes/other');
  await activeRoute(page, 'unrelated-page');
  expect(other.data.buildId).toBe(initial.buildId);
  await expect(page.locator('preview-page')).toHaveCount(0);
  await expect(page.locator('unrelated-page .unrelated')).toBeVisible();
  expect(requests).toContain(behavior(graph, 'unrelated-page'));
  expect(requests).not.toContain(behavior(graph, 'unrelated-panel'));
  const epoch = await page.evaluate(() => window.__webui?.inventoryEpoch ?? 0);
  await page.evaluate(() =>
    (window as unknown as { lazyRouteRouter: { gc(): void } }).lazyRouteRouter.gc(),
  );
  await expect(page.locator('unrelated-page .unrelated')).toBeVisible();
  expect(await page.evaluate(() => window.__webui?.inventoryEpoch)).toBe(epoch + 1);
  const returning = await navigate(page, '/lazy-routes/preview');
  await activeRoute(page, 'preview-page');
  expect(returning.data.templates).toHaveProperty('preview-page');
  expect(returning.data.buildId).toBe(initial.buildId);
  await expect(page.locator('body')).toHaveAttribute('data-document', 'original');
  await expect(page.locator('help-panel')).toHaveCount(0);
  await page.locator('preview-page .preview').hover();
  await expect(page.locator('help-panel p')).toHaveText('Page-owned value');
  await expect(page.locator('help-panel p')).toHaveCSS('color', 'rgb(17, 81, 139)');
  await page.locator('preview-page .owner-event').click();
  await expect(page.locator('preview-page .owner-event')).toHaveText('1');
  for (const tag of ['preview-page', 'help-panel', 'shared-panel']) {
    expect(requests.filter(path => path === behavior(graph, tag))).toHaveLength(1);
  }
  expect(requests).not.toContain(behavior(graph, 'nested-panel'));
  expect(requests).not.toContain(behavior(graph, 'sibling-panel'));
  expect(requests).not.toContain(behavior(graph, 'unrelated-panel'));
  for (const index of [1, 2, 3, 4]) expect(requests).not.toContain(pack(graph, 'preview-page', index));
  expect(requests).not.toContain(pack(graph, 'unrelated-page', 0));
  expect(errors).toEqual([]);
});

test('real router: direct SSR preview resolves its page loader without fetching closed bodies', async ({ page }) => {
  const { graph, requests } = await observe(page, 'routed');
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  const document = await page.goto('/lazy-routes/preview');
  if (!document) throw new Error('Missing SSR document response.');
  await activeRoute(page, 'preview-page');
  await expect(page.locator('preview-page .preview')).toBeVisible();
  expect(bootstrap(await document.text()).chain)
    .toEqual(['routed-shell', 'preview-page']);
  expect(requests).toContain(behavior(graph, 'preview-page'));
  expect(requests).not.toContain(behavior(graph, 'help-panel'));
  expect(requests).not.toContain(behavior(graph, 'unrelated-page'));
  expect(requests.filter(path => path.includes('/_webui/packs/'))).toEqual([]);
  await page.locator('preview-page .preview').hover();
  await expect(page.locator('help-panel p')).toHaveText('Page-owned value');
  expect(errors).toEqual([]);
});
