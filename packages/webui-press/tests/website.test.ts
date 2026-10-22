// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { docsDist as dist, startWebsite } from './website-fixture.js';

test('the public homepage renders without JavaScript and enhances its real example', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const noScript = await browser.newPage({ javaScriptEnabled: false });
  await noScript.goto(`${server.origin}/webui/`);
  await expect(noScript.getByRole('heading', { level: 1 })).toHaveText('The Modern Web, Without the Weight');
  await expect(noScript).toHaveTitle('The Modern Web, Without the Weight | WebUI Framework');
  await expect(noScript.getByRole('link', { name: 'WebUI Framework home', exact: true })).toBeVisible();
  await expect(noScript.locator('home-counter output')).toHaveText('0');
  await expect(noScript.getByRole('button', { name: 'Add one', exact: true })).toBeDisabled();
  await expect(noScript.locator('rendering-diagram svg')).toHaveCount(3);
  await expect(noScript.getByRole('link', { name: 'Build your first page', exact: true }))
    .toHaveAttribute('href', '/webui/tutorials/hello-world/');
  await expect(noScript.locator('.host-list').getByRole('link', { name: 'Node / Bun', exact: true }))
    .toHaveAttribute('href', '/webui/guide/integrations/node/');
  await expect(noScript.locator('.host-list')).not.toContainText(/deno/i);
  const ssrHeight = (await noScript.locator('home-counter').boundingBox())?.height;
  assert.ok(ssrHeight);
  await noScript.close();

  const page = await browser.newPage();
  await page.route('https://**', route => route.abort());
  const errors: string[] = [];
  const scriptRequests = new Map<string, number>();
  const wasmRequests: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => {
    const url = new URL(request.url());
    if (url.origin === server.origin && url.pathname.endsWith('.js')) {
      scriptRequests.set(url.pathname, (scriptRequests.get(url.pathname) ?? 0) + 1);
    }
    if (url.pathname.endsWith('.wasm')) wasmRequests.push(request.url());
  });
  await page.goto(`${server.origin}/webui/`);
  const add = page.getByRole('button', { name: 'Add one', exact: true });
  await expect(add).toBeEnabled();
  const hydratedHeight = (await page.locator('home-counter').boundingBox())?.height;
  assert.ok(hydratedHeight);
  assert.ok(Math.abs(hydratedHeight - ssrHeight) <= 1, 'Hydration must not shift the demo layout.');
  await add.click();
  await expect(page.locator('home-counter output')).toHaveText('1');
  await add.click();
  await expect(page.locator('home-counter output')).toHaveText('2');
  await page.getByRole('button', { name: 'Reset', exact: true }).click();
  await expect(page.locator('home-counter output')).toHaveText('0');
  await expect(page.getByRole('button', { name: 'Reset', exact: true })).toBeDisabled();
  assert.deepEqual(errors, []);
  assert.deepEqual([...scriptRequests].filter(([, count]) => count > 1), [],
    'Preloads and imports must fetch each script URL identity once.');
  assert.ok(![...scriptRequests.keys()].some(url => /playground|codemirror|wasm/i.test(url)),
    'The homepage must not fetch the playground runtime.');
  assert.deepEqual(wasmRequests, [], 'Describing WASM must not load a WASM bundle on the homepage.');
});

test('the homepage explains streaming, worker rendering, WASM, and static generation without JavaScript', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const capabilities = [
    ['Streaming SSR', 'Streaming SSR', 'guide/concepts/directives/boundary/', 'Hydrate completed regions'],
    ['Service workers', 'Service-worker rendering', 'guide/serverless-architecture/', 'caching policy and first-load bootstrap'],
    ['WebAssembly', 'WebAssembly (WASM)', 'guide/integrations/wasm/', 'handler-only, parser-only'],
    ['Static sites', 'Static site generation', 'guide/webui-press/', 'WebUI Press builds Markdown'],
  ];
  for (const width of [1440, 390, 320]) {
    const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width, height: 900 } });
    await page.goto(`${server.origin}/webui/`);
    const shortcuts = page.getByRole('navigation', { name: 'Rendering and deployment guides' });
    await expect(shortcuts.getByRole('link')).toHaveCount(5);
    await expect(page.locator('.delivery-list > div')).toHaveCount(4);
    for (const [shortcut, label, target, detail] of capabilities) {
      await expect(shortcuts.getByRole('link', { name: shortcut, exact: true })).toHaveAttribute('href', `/webui/${target}`);
      const row = page.locator('.delivery-list > div').filter({ has: page.getByRole('link', { name: label, exact: true }) });
      await expect(row.locator('a')).toHaveAttribute('href', `/webui/${target}`);
      await expect(row.locator('dd')).toContainText(detail);
    }
    await expect(shortcuts.getByRole('link', { name: 'Component assets', exact: true }))
      .toHaveAttribute('href', '/webui/guide/concepts/interactivity/#loading-static-component-assets');
    const assets = page.locator('section[aria-labelledby="component-assets-title"]');
    await expect(assets.getByRole('heading')).toHaveText('Load component assets on demand');
    await expect(assets.locator('code')).toContainText('--emit-component-assets settings-dialog');
    await expect(assets.locator('figcaption')).toContainText('For non-routed UI');
    await expect(assets.getByRole('link', { name: 'Build and load component assets' }))
      .toHaveAttribute('href', '/webui/guide/concepts/interactivity/#loading-static-component-assets');
    const overflow = await page.locator('.main-content').evaluate(element => element.scrollWidth - element.clientWidth);
    assert.ok(overflow <= 1, `${width}px: capability descriptions must not overflow.`);
    await page.close();
  }
});

test('current runtime guidance keeps Node.js and Bun without Deno recommendations', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ javaScriptEnabled: false });
  await page.goto(`${server.origin}/webui/guide/integrations/node/`);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Node.js and Bun #');
  await expect(page.locator('#nodejs-bun-and-deno')).toHaveCount(1);
  await expect(page.locator('webui-press-tab[slot="tab"]')).toHaveText(['Node.js', 'Bun']);
  await expect(page.locator('webui-press-tab-panel')).toHaveCount(2);
  await expect(page.getByRole('link', { name: 'Node / Bun', exact: true })).toBeVisible();
  await expect(page.locator('.doc-content')).not.toContainText(/deno/i);
  for (const pathname of ['guide/integrations/', 'guide/concepts/performance/']) {
    await page.goto(`${server.origin}/webui/${pathname}`);
    await expect(page.locator('.doc-content')).not.toContainText(/deno/i);
  }
});

test('site navigation, skip links and reading layouts work at narrow widths', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
  await page.route('https://**', route => route.abort());

  for (const pathname of ['guide/', 'tutorials/hello-world/', 'examples/']) {
    await page.goto(`${server.origin}/webui/${pathname}`);
    const current = new URL(page.url());
    const skipTarget = await page.locator('.skip-link').evaluate(
      element => new URL((element as HTMLAnchorElement).href).pathname,
    );
    assert.equal(skipTarget, current.pathname, 'Skip link must stay on the current document.');
    const dimensions = await page.locator('.main-content').evaluate(element => ({
      scroll: element.scrollWidth,
      client: element.clientWidth,
    }));
    assert.ok(dimensions.scroll <= dimensions.client + 1, `${pathname} must not overflow horizontally.`);
    await expect(page.locator('.logo span')).toHaveText('WebUI Framework');
  }

  const menu = page.getByRole('button', { name: 'Open navigation' });
  await menu.click();
  await expect(page.getByRole('dialog', { name: 'Browse WebUI' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: 'Browse WebUI' })).not.toBeVisible();
  await expect(menu).toBeFocused();
});

test('benchmark evidence remains useful without JavaScript', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 390, height: 844 } });
  await page.goto(`${server.origin}/webui/benchmarks/`);
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  await expect(page.locator('benchmark-explorer')).toBeVisible();
  assert.ok(await page.locator('benchmark-explorer svg').count() > 0, 'Measurements need an SSR SVG figure.');
  assert.ok(await page.locator('benchmark-explorer table').count() > 0, 'Measurements need an equivalent data table.');
  await expect(page.getByText(/historical/i).first()).toBeVisible();
  const dimensions = await page.locator('.main-content').evaluate(element => ({
    scroll: element.scrollWidth,
    client: element.clientWidth,
  }));
  assert.ok(dimensions.scroll <= dimensions.client + 1, 'Benchmark page must not overflow the viewport.');
});

test('published pages retain clean headings, valid internal links, anchors and distinct metadata', async t => {
  const htmlFiles: string[] = [];
  const directories = [dist];
  while (directories.length) {
    const directory = directories.pop();
    assert.ok(directory);
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const file = path.join(directory, entry.name);
      if (entry.isDirectory()) directories.push(file);
      else if (entry.name.endsWith('.html')) htmlFiles.push(file);
    }
  }

  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ javaScriptEnabled: false });
  const ids = new Map<string, Set<string>>();
  const pages: { relative: string; links: string[] }[] = [];
  for (const file of htmlFiles) {
    const relative = path.relative(dist, file).split(path.sep).join('/');
    if (relative === '404.html') continue;
    await page.goto(`${server.origin}/webui/${relative.replace(/index\.html$/, '')}`);
    const result = await page.evaluate(() => {
      const roots: (Document | ShadowRoot)[] = [document];
      const links: string[] = [];
      const ids: string[] = [];
      const headings: string[] = [];
      while (roots.length) {
        const root = roots.pop();
        if (!root) throw new Error('Missing DOM root');
        for (const element of root.querySelectorAll('*')) {
          if (element.id) ids.push(element.id);
          if (element instanceof HTMLHeadingElement) {
            headings.push(Array.from(element.childNodes, node =>
              node instanceof Element && node.matches('.header-anchor') ? '' : node.textContent ?? '',
            ).join('').trim());
          }
          if ((element instanceof HTMLAnchorElement || element instanceof HTMLLinkElement) && element.hasAttribute('href')) {
            links.push(element.href);
          }
          if (element.shadowRoot) roots.push(element.shadowRoot);
        }
      }
      return {
        title: document.title,
        description: document.querySelector('meta[name="description"]')?.getAttribute('content'),
        canonical: document.querySelector('link[rel="canonical"]')?.getAttribute('href'),
        links,
        ids,
        headings,
      };
    });
    assert.ok(result.title && result.title !== 'WebUI Framework | WebUI Framework', `${relative}: use a meaningful document title.`);
    assert.ok(result.title === 'WebUI Framework' || result.title.endsWith(' | WebUI Framework'),
      `${relative}: preserve the full framework name in document titles.`);
    assert.ok(result.description, `${relative}: missing description.`);
    assert.ok(result.canonical, `${relative}: missing canonical URL.`);
    for (const heading of result.headings) {
      assert.ok(!heading.endsWith('.'), `${relative}: remove the heading's trailing period: ${heading}`);
    }
    ids.set(file, new Set(result.ids));
    pages.push({ relative, links: result.links });
  }

  const failures = new Set<string>();
  for (const { relative, links } of pages) {
    for (const href of links) {
      const url = new URL(href);
      if (![server.origin, 'https://microsoft.github.io'].includes(url.origin) || !url.pathname.startsWith('/webui/')) continue;
      const target = path.join(dist, decodeURIComponent(url.pathname.slice('/webui/'.length)));
      const destination = path.extname(target) ? target : path.join(target, 'index.html');
      if (!fs.existsSync(destination)) {
        failures.add(`${relative}: missing ${url.pathname}`);
      } else if (url.hash && ids.has(destination)) {
        const fragment = decodeURIComponent(url.hash.slice(1));
        if (!ids.get(destination)?.has(fragment)) {
          failures.add(`${relative}: missing ${url.pathname}${url.hash}`);
        }
      }
    }
  }
  assert.equal(failures.size, 0,
    `Every built internal link and fragment must resolve. First failures:\n${[...failures].slice(0, 40).join('\n')}`);
});
