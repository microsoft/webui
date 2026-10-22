// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import type { Page } from '@playwright/test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import test from 'node:test';
import type { TestContext } from 'node:test';
import { build, fixture, removeFixtureRoot, write } from './native-fixture.js';

const basePath = '/nested/press/';
const title = 'Press foundation';

async function foundationSite(t: TestContext, mode: 'all' | 'content') {
  const f = fixture();
  const output = path.join(f.site, 'dist');
  let server: http.Server | undefined;
  t.after(async () => {
    if (server) {
      server.closeAllConnections();
      await new Promise<void>(resolve => server?.close(() => resolve()));
    }
    removeFixtureRoot(f.root);
  });
  write(path.join(f.site, 'content/index.md'), `---\nlayout: home\ntitle: ${title}\n---\n# Home`);
  const doc = path.join(f.site, 'content/doc.md');
  write(doc, fs.readFileSync(doc, 'utf8') + `
### Details

More API details.

## \`<for>\` & labels

<nav aria-label="Authored headings">
<for each="heading in page.headings">
<a href="{{page.path}}#{{heading.anchor}}" data-level="{{heading.level}}">{{heading.text}}</a>
</for>
</nav>
`);
  write(f.configFile, JSON.stringify({
    ...f.config,
    basePath,
    site: {
      title,
      description: 'Site summary',
      ...(mode === 'all' ? { url: 'https://example.test/' } : {}),
    },
    customPages: {
      ...f.config.customPages,
      '/custom': {
        ...f.config.customPages['/custom'],
        title: 'Playground & templates',
        description: 'Try <templates> without installing tools.',
      },
      '/legacy': '<h1>Legacy custom page</h1>',
    },
    regions: {
      'doc.beforeContent': {
        html: `<nav aria-label="On this page">
          <for each="heading in page.headings">
            <if condition="heading.level > 1">
              <a href="{{page.path}}#{{heading.anchor}}" data-level="{{heading.level}}">{{heading.text}}</a>
            </if>
          </for>
        </nav>`,
      },
    },
  }));
  build(f.site, `--show=${mode}`);
  const mime: Record<string, string> = {
    '.css': 'text/css', '.js': 'text/javascript', '.json': 'application/json', '.svg': 'image/svg+xml',
  };
  server = http.createServer((request, response) => {
    const url = new URL(request.url ?? '/', 'http://localhost');
    const relative = url.pathname.startsWith(basePath) ? url.pathname.slice(basePath.length) : '';
    let file = path.resolve(output, decodeURIComponent(relative));
    if (!file.startsWith(output + path.sep) && file !== output) {
      response.writeHead(403).end();
      return;
    }
    if (fs.existsSync(file) && fs.statSync(file).isDirectory()) file = path.join(file, 'index.html');
    if (!fs.existsSync(file)) file = path.join(output, '404.html');
    response.setHeader('Content-Type', mime[path.extname(file)] ?? 'text/html');
    if (path.extname(file) === '.js') {
      response.setHeader('Cache-Control', 'public, max-age=31536000, immutable');
    }
    fs.createReadStream(file).pipe(response);
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const address = server.address();
  assert.ok(address && typeof address !== 'string');
  return { ...f, output, origin: `http://127.0.0.1:${address.port}` };
}

async function expectHeadingLinks(page: Page, selector: string): Promise<void> {
  const links = page.locator(selector).getByRole('link');
  assert.equal(await links.count(), 4);
  for (const link of await links.all()) {
    const href = await link.getAttribute('href');
    assert.ok(href, 'Missing heading URL');
    assert.ok(href.startsWith(`${basePath}doc/#`), href);
    const anchor = href.slice(href.indexOf('#') + 1);
    const heading = page.locator(`[id="${anchor}"]`);
    await expect(heading).toHaveCount(1);
    const level = await link.getAttribute('data-level');
    assert.equal(await heading.evaluate(element => element.tagName.toLowerCase()), `h${level}`);
  }
  await expect(links.last()).toHaveText('<for> & labels');
}

test('full Press output preserves module identity, document metadata and native navigation', async t => {
  const f = await foundationSite(t, 'all');
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));

  await t.test('preloads and imported chunks use one URL, retaining early loads and cache invalidation', async () => {
    await page.goto(`${f.origin}${basePath}doc/`, { waitUntil: 'networkidle' });
    const preloads = await page.locator('link[rel="modulepreload"]').evaluateAll(links =>
      links.map(link => (link as HTMLLinkElement).href),
    );
    assert.ok(preloads.length > 0, 'Static dependencies must still preload');
    const resources = await page.evaluate(() =>
      performance.getEntriesByType('resource').map(entry => entry.name)
        .filter(name => new URL(name).pathname.endsWith('.js')),
    );
    const byPath = new Map<string, string[]>();
    for (const name of resources) {
      const pathname = new URL(name).pathname;
      const urls = byPath.get(pathname) ?? [];
      urls.push(name);
      byPath.set(pathname, urls);
    }
    for (const [pathname, urls] of byPath) {
      assert.equal(urls.length, 1, `${pathname} was requested more than once: ${urls.join(', ')}`);
    }
    assert.ok(preloads.some(url => /\/assets\/chunk-[^/]+\.js$/.test(url)),
      'Content-hashed dependencies must use the exact unversioned import URL');
    const entryBefore = await page.locator('script[type="module"][src]').evaluateAll(scripts =>
      scripts.map(script => (script as HTMLScriptElement).src),
    );
    assert.ok(entryBefore.some(url => /\/index\.js\?v=[a-f0-9]+$/.test(url)),
      'Stable root entries must retain content versioning');
    write(path.join(f.site, '.webui-press/components/fixture-preview/preview.ts'),
      'document.documentElement.dataset.galleryAlias = "rebuilt";');
    build(f.site, '--show=all');
    await page.reload({ waitUntil: 'networkidle' });
    await expect(page.locator('html')).toHaveAttribute('data-gallery-alias', 'rebuilt');
    const entryAfter = await page.locator('script[type="module"][src]').evaluateAll(scripts =>
      scripts.map(script => (script as HTMLScriptElement).src),
    );
    assert.notDeepEqual(entryBefore, entryAfter, 'Changed browser code must invalidate its entry URL');
  });

  await t.test('titles and canonical metadata work for home, Markdown, custom pages and 404', async () => {
    for (const [route, expectedTitle, description] of [
      ['', title, 'Site summary'],
      ['doc/', `doc example | ${title}`, 'Example metadata'],
      ['custom/', `Playground & templates | ${title}`, 'Try <templates> without installing tools.'],
      ['legacy/', title, 'Site summary'],
    ]) {
      await page.goto(`${f.origin}${basePath}${route}`);
      await expect(page).toHaveTitle(expectedTitle);
      await expect(page.locator('meta[property="og:title"]')).toHaveAttribute('content', expectedTitle);
      await expect(page.locator('meta[name="description"]')).toHaveAttribute('content', description);
      await expect(page.locator('meta[property="og:description"]')).toHaveAttribute('content', description);
      const canonical = `https://example.test${basePath}${route}`;
      await expect(page.locator('link[rel="canonical"]')).toHaveAttribute('href', canonical);
      await expect(page.locator('meta[property="og:url"]')).toHaveAttribute('content', canonical);
    }
    await page.goto(`${f.origin}${basePath}missing/`);
    await expect(page).toHaveTitle(`Page Not Found | ${title}`);
    await expect(page.locator('link[rel="canonical"], meta[property="og:url"]')).toHaveCount(0);
  });

  await t.test('headings are available to site regions and match search anchors without JavaScript', async () => {
    const noJs = await browser.newPage({ javaScriptEnabled: false });
    try {
      await noJs.goto(`${f.origin}${basePath}doc/`);
      await expectHeadingLinks(noJs, 'nav[aria-label="Authored headings"]');
      const toc = noJs.getByRole('navigation', { name: 'On this page' });
      await expect(toc.getByRole('link')).toHaveCount(3);
      const index = JSON.parse(fs.readFileSync(path.join(f.output, 'search-index.json'), 'utf8')) as {
        path: string; headings: { text: string; anchor: string; level: number }[];
      }[];
      const entry = index.find(item => item.path === `${basePath}doc`);
      assert.ok(entry);
      const links = await noJs.getByRole('navigation', { name: 'Authored headings' })
        .getByRole('link').evaluateAll(anchors => anchors.map(anchor => ({
          text: anchor.textContent,
          anchor: (anchor as HTMLAnchorElement).hash.slice(1),
          level: Number(anchor.getAttribute('data-level')),
        })));
      assert.deepEqual(links, entry.headings);
      await toc.getByRole('link', { name: 'API', exact: true }).click();
      assert.equal(new URL(noJs.url()).hash, '#api');
    } finally {
      await noJs.close();
    }
  });

  await t.test('skip navigation stays on the current document, including aliases, query strings and 404', async () => {
    for (const route of ['doc/', 'doc?view=compact', 'custom/?view=preview', 'missing/']) {
      const url = `${f.origin}${basePath}${route}`;
      await page.goto(url);
      const skip = page.getByRole('link', { name: 'Skip to content' });
      await expect(skip).toHaveAttribute('href', `${new URL(url).pathname}${new URL(url).search}#main-content`);
      const documents: string[] = [];
      const track = (request: import('@playwright/test').Request) => {
        if (request.isNavigationRequest() && request.frame() === page.mainFrame()) documents.push(request.url());
      };
      page.on('request', track);
      await skip.focus();
      await page.keyboard.press('Enter');
      await expect(page.locator('#main-content')).toBeFocused();
      assert.equal(page.url(), `${url}#main-content`);
      assert.deepEqual(documents, [], 'Skip navigation must not request another document');
      page.off('request', track);
    }
    const noJs = await browser.newPage({ javaScriptEnabled: false });
    try {
      await noJs.goto(`${f.origin}${basePath}doc/`);
      await noJs.getByRole('link', { name: 'Skip to content' }).focus();
      await noJs.keyboard.press('Enter');
      assert.equal(noJs.url(), `${f.origin}${basePath}doc/#main-content`);
      await expect(noJs.locator('#main-content')).toBeFocused();
    } finally {
      await noJs.close();
    }
  });

  await t.test('search has accessible names, both shortcuts, Escape focus restoration and working anchors', async () => {
    await page.goto(`${f.origin}${basePath}doc/`);
    const search = page.locator('docs-search');
    const trigger = search.getByRole('button', { name: 'Search', exact: true });
    await expect(search.locator('kbd')).toHaveText('Ctrl/⌘ K');
    await expect(trigger).toHaveAttribute('aria-keyshortcuts', 'Control+K Meta+K');
    const loaded = page.waitForResponse(response => response.url().endsWith('search-index.json'));
    await trigger.click();
    await loaded;
    const dialog = search.getByRole('dialog', { name: 'Search documentation' });
    const input = dialog.getByRole('textbox', { name: 'Search documentation' });
    await expect(dialog).toBeVisible();
    await expect(input).toBeFocused();
    await input.fill('API');
    await expect(dialog.getByRole('link', { name: /doc example > API/ })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(dialog).not.toBeVisible();
    await expect(trigger).toBeFocused();
    for (const shortcut of ['Control+k', 'Meta+k']) {
      await page.keyboard.press(shortcut);
      await expect(dialog).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(trigger).toBeFocused();
    }
    await trigger.click();
    await input.fill('API');
    await dialog.getByRole('link', { name: /doc example > API/ }).click();
    await expect(page).toHaveURL(`${f.origin}${basePath}doc#api`);
    await page.goBack();
    await expect(page).toHaveURL(`${f.origin}${basePath}doc/`);
  });
  assert.deepEqual(errors, []);
});

test('content-only output retains heading and title metadata without opting into canonical URLs or shell', async t => {
  const f = await foundationSite(t, 'content');
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ javaScriptEnabled: false });
  await page.goto(`${f.origin}${basePath}doc/`);
  await expectHeadingLinks(page, 'nav[aria-label="Authored headings"]');
  await expect(page.locator('docs-search, .skip-link, nav[aria-label="On this page"]')).toHaveCount(0);
  await expect(page.locator('link[rel="canonical"], meta[property="og:url"]')).toHaveCount(0);
  await expect(page).toHaveTitle(`doc example | ${title}`);
  await page.goto(`${f.origin}${basePath}`);
  await expect(page).toHaveTitle(title);
  await page.goto(`${f.origin}${basePath}custom/`);
  await expect(page).toHaveTitle(`Playground & templates | ${title}`);
  const config = JSON.parse(fs.readFileSync(f.configFile, 'utf8'));
  config.site.url = 'https://example.test';
  write(f.configFile, JSON.stringify(config));
  build(f.site, '--show=content');
  await page.reload();
  await expect(page.locator('link[rel="canonical"]'))
    .toHaveAttribute('href', `https://example.test${basePath}custom/`);
  await expect(page.locator('meta[property="og:url"]'))
    .toHaveAttribute('content', `https://example.test${basePath}custom/`);
});
