// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import type { Browser } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import test from 'node:test';
import { binary, build, fixture, removeFixtureRoot, write } from './native-fixture.js';

const shell = '.nav-bar, docs-site-navigation, docs-sidebar-navigation, docs-search, ' +
  'docs-theme-toggle, .sidebar, .mobile-page-context, .page-nav, .site-footer, .home-hero';

function pageSnapshot(html: string): { document: string; bootstrap: object; modules: string[] } {
  // Compare bootstrap objects semantically, not by published 0.0.28's
  // unordered serialization/registration. CSS closures and document markup
  // stay ordered; bundled module identities are compared separately.
  const marker = '<script type="application/json" id="webui-data">';
  const start = html.indexOf(marker) + marker.length;
  assert.ok(start >= marker.length);
  const end = html.indexOf('</script>', start);
  assert.ok(end > start);
  const data: unknown = JSON.parse(html.slice(start, end));
  assert.ok(data && typeof data === 'object' && 'css' in data && Array.isArray(data.css));
  data.css.sort();
  return {
    document: html.slice(0, start),
    bootstrap: data,
    modules: Array.from(html.matchAll(/<script type="module" src="([^"]+)"/g), match => match[1]),
  };
}

test('native builds retain the actionable parser error beneath page context', () => {
  const f = fixture();
  try {
    write(path.join(f.site, 'content/doc.md'), '# Invalid example\n\n<input :value="{{count}}">');
    const result = spawnSync(binary, ['build', '--show=content'], {
      cwd: f.site, encoding: 'utf8', timeout: 60_000,
    });
    assert.equal(result.error, undefined);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /\/fixture\/doc: Failed to parse index.html/);
    assert.match(result.stderr, /:value complex binding is only allowed on custom elements/);
    assert.match(result.stderr, /Use value=/);

    write(path.join(f.site, 'content/doc.md'), '# Invalid nesting\n\n<div><test-catalog-text></div>');
    const nesting = spawnSync(binary, ['build', '--show=content'], {
      cwd: f.site, encoding: 'utf8', timeout: 60_000,
    });
    assert.equal(nesting.error, undefined);
    assert.equal(nesting.status, 1);
    assert.match(nesting.stderr, /\[unclosed-html-tag\]/);
    assert.match(nesting.stderr, /index.html:\d+:\d+/);
    assert.match(nesting.stderr, /help:/);
  } finally {
    removeFixtureRoot(f.root);
  }
});

test('native content builds preserve all page layouts and npm/local authored content', () => {
  const f = fixture();
  try {
    build(f.site, '--config', '.webui-press/config.json', '--show=content');
    for (const page of ['doc', 'page', 'full', 'home', 'custom', '404']) {
      const file = page === '404' ? '404.html' : `${page}/index.html`;
      const html = fs.readFileSync(path.join(f.site, 'dist', file), 'utf8');
      assert.match(html, /<main [^>]*id="main-content"/);
      assert.match(html, /<article class="doc-content">/);
      assert.match(html, /name="fixture-head"/);
      for (const absent of ['<docs-site-navigation', '<docs-sidebar-navigation', '<docs-search',
        'class="nav-bar"', 'class="sidebar"', 'class="page-nav"', 'class="site-footer"',
        'mobile-page-context', 'webui-press:sidebar-scroll', 'Shell announcement']) {
        assert.ok(!html.includes(absent), `${file} contains ${absent}`);
      }
      if (page !== '404') {
        assert.match(html, /<edge-hub-header/);
        assert.match(html, /<edge-side-pane/);
        assert.match(html, /Authored header example/);
        assert.match(html, /Authored side pane example/);
        assert.match(html, /<test-catalog-button/);
        assert.match(html, /Preview[^<]*2/);
        assert.match(html, /Slotted example/);
        assert.match(html, /<test-catalog-text/);
        assert.match(html, /Static catalog content/);
        assert.match(html, /Static slot/);
        assert.match(html, /<script type="module" src=/);
      }
    }
    const css = fs.readFileSync(path.join(f.site, 'dist/docs.css'), 'utf8');
    assert.ok(!css.includes('overflow: hidden'));
    assert.ok(!css.includes('.main-content'));
    assert.ok(!css.includes('.sidebar'));
    assert.ok(!css.includes('body[data-layout="full"]'));

    const relativeConfig = pageSnapshot(fs.readFileSync(path.join(f.site, 'dist/doc/index.html'), 'utf8'));
    build(f.site, '--config', f.configFile, '--show=content');
    assert.deepEqual(
      pageSnapshot(fs.readFileSync(path.join(f.site, 'dist/doc/index.html'), 'utf8')),
      relativeConfig,
    );

    // The omitted option must still match explicit all, including home behavior.
    write(f.configFile, JSON.stringify({ ...f.config, regions: {} }));
    build(f.site);
    const implicit = fs.readFileSync(path.join(f.site, 'dist/doc/index.html'), 'utf8');
    assert.match(implicit, /<docs-site-navigation/);
    assert.match(implicit, /class="site-footer"/);
    const home = fs.readFileSync(path.join(f.site, 'dist/home/index.html'), 'utf8');
    assert.ok(!home.includes('<test-catalog-button'));
    write(f.configFile, JSON.stringify({ ...f.config, regions: {}, show: 'content' }));
    build(f.site, '--show=all');
    assert.deepEqual(
      pageSnapshot(fs.readFileSync(path.join(f.site, 'dist/doc/index.html'), 'utf8')),
      pageSnapshot(implicit),
    );
  } finally {
    removeFixtureRoot(f.root);
  }
});

test('native content serve hydrates SSR nodes and keeps the override after config reload', async (t) => {
  const f = fixture();
  const port = await unusedPort();
  const child = spawn(binary, ['serve', '--show=content', '--port', String(port)], {
    cwd: f.site, stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', (data) => { output += data; });
  child.stderr.on('data', (data) => { output += data; });
  const exited = once(child, 'exit');
  let browser: Browser | undefined;
  t.after(async () => {
    await browser?.close();
    child.kill('SIGINT');
    await exited;
    removeFixtureRoot(f.root);
  });
  const origin = `http://127.0.0.1:${port}/fixture/`;
  await expect.poll(async () => {
    assert.equal(child.exitCode, null, output);
    try {
      const response = await fetch(`${origin}doc/`, { headers: { Connection: 'close' } });
      await response.arrayBuffer();
      return response.status;
    }
    catch (error) {
      if (error instanceof TypeError && error.cause instanceof Error &&
          'code' in error.cause && error.cause.code === 'ECONNREFUSED') return 0;
      throw error;
    }
  }, { timeout: 60_000 }).toBe(200);
  browser = await chromium.launch({ headless: true });
  const noJs = await browser.newPage({ javaScriptEnabled: false });
  await noJs.goto(`${origin}doc/`);
  await expect(noJs.locator(shell)).toHaveCount(0);
  await expect(noJs.getByRole('button', { name: 'Preview 2' })).toBeVisible();
  await expect(noJs.locator('test-catalog-text')).toContainText('Static slot');
  await expect(noJs.locator('test-catalog-text span')).toContainText('Static catalog content');
  await expect(noJs.getByRole('heading', { name: /^API\b/, level: 2 })).toBeVisible();

  const page = await browser.newPage();
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => {
    if (message.type() === 'warning') errors.push(message.text());
  });
  let releaseScripts: () => void = () => {};
  const scriptsReady = new Promise<void>(resolve => { releaseScripts = resolve; });
  await page.route('**/assets/*.js*', async route => {
    await scriptsReady;
    await route.continue();
  });
  await page.goto(`${origin}doc/`, { waitUntil: 'commit' });
  const button = await page.getByRole('button', { name: 'Preview 2' }).elementHandle();
  assert.ok(button);
  releaseScripts();
  await page.waitForFunction(() =>
    (document.querySelector('test-catalog-button') as HTMLElement & { $ready?: boolean })?.$ready);
  await expect(page.locator('html')).toHaveAttribute('data-gallery-alias', 'ready');
  assert.ok(await button.evaluate(el =>
    el === document.querySelector('test-catalog-button')?.shadowRoot?.querySelector('button')));
  const hydrated = await page.locator('test-catalog-button').evaluate(host => ({
    count: (host as HTMLElement & { count: number }).count,
    state: document.querySelector('#webui-data')?.textContent,
  }));
  assert.equal(hydrated.count, 2, JSON.stringify(hydrated));
  await button.click();
  await expect(page.getByRole('button', { name: 'Preview 3' })).toBeVisible();
  for (const viewport of [{ width: 1440, height: 1000 }, { width: 390, height: 844 }]) {
    await page.setViewportSize(viewport);
    for (const route of ['doc', 'page', 'full', 'home', 'custom', 'missing']) {
      await page.goto(`${origin}${route}/`);
      await expect(page.locator(shell)).toHaveCount(0);
      const layout = await page.evaluate(() => {
        const article = document.querySelector('main > article');
        if (!article) throw new Error('article is missing');
        return {
          x: article.getBoundingClientRect().x,
          width: article.getBoundingClientRect().width,
          maxWidth: getComputedStyle(article).maxWidth,
          overflow: getComputedStyle(document.body).overflowY,
          documentOverflow: getComputedStyle(document.documentElement).overflowY,
          scrollWidth: document.documentElement.scrollWidth,
        };
      });
      assert.equal(layout.x, 16);
      assert.equal(layout.width, viewport.width - 32);
      assert.equal(layout.maxWidth, 'none');
      assert.equal(layout.overflow, 'visible');
      assert.equal(layout.documentOverflow, 'visible');
      assert.ok(layout.scrollWidth <= viewport.width);
      if (route === 'doc' && process.env.WEBUI_PRESS_SCREENSHOTS) {
        fs.mkdirSync(process.env.WEBUI_PRESS_SCREENSHOTS, { recursive: true });
        await page.screenshot({
          path: path.join(process.env.WEBUI_PRESS_SCREENSHOTS, `content-gallery-${viewport.width}.png`),
          fullPage: true,
        });
      }
    }
  }
  write(f.configFile, JSON.stringify({ ...f.config, show: 'all', site: { title: 'Reloaded gallery' } }));
  await expect.poll(async () =>
    (await fetch(`${origin}doc/`, { headers: { Connection: 'close' } })).text(),
    { timeout: 30_000 }).toContain('Reloaded gallery');
  await page.goto(`${origin}doc/`);
  await expect(page.locator(shell)).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Preview 2' })).toBeVisible();
  assert.deepEqual(errors, []);
});

async function unusedPort(): Promise<number> {
  const server = net.createServer();
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const address = server.address();
  assert.ok(address && typeof address !== 'string');
  await new Promise<void>(resolve => server.close(() => resolve()));
  return address.port;
}
