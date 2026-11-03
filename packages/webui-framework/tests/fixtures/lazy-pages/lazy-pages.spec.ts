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

async function setup(page: Page): Promise<{ graph: Graph; requests: string[] }> {
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
  const requests: string[] = [];
  page.on('request', request => requests.push(new URL(request.url()).pathname));
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
  expect(requests.filter(path => path.includes('/_webui/packs/'))).toEqual([]);
  await page.locator('pages-shell .first').click();
  await expect(page.locator('preview-page .preview')).toBeVisible();
  expect(requests).toContain(behavior(graph, 'preview-page'));
  expect(requests).not.toContain(behavior(graph, 'help-panel'));
  expect(requests).not.toContain(behavior(graph, 'unrelated-page'));
  expect(requests.filter(path => path.includes('/_webui/packs/'))).toEqual([]);
  return { graph, requests };
}

test('hover activates only its comparison region while pack, code and Link CSS overlap', async ({ page }) => {
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

test('selective idle and path prefetch warm without mounting or fetching application state', async ({ page }) => {
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

test('rapid close cannot mount a completed load; later activation reuses resources', async ({ page }) => {
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
