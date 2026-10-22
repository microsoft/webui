// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import type { Page } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import type { TestContext } from 'node:test';
import { build, fixture, removeFixtureRoot, write } from './native-fixture.js';
import { startWebsite } from './website-fixture.js';

const base = '/nested/outline/';
declare global {
  interface Window {
    testOutlineSignals: AbortSignal[];
  }
}

const body = '# Example\n\n' + Array.from({ length: 36 }, (_, index) =>
  `## Section ${index + 1}\n\n${'A paragraph describing this section.\n\n'.repeat(index % 5 + 2)}`,
).join('\n');

async function outlineSite(
  t: TestContext,
  outline?: boolean,
  mode: 'all' | 'content' = 'all',
  replacement?: string,
) {
  const f = fixture();
  let server: Awaited<ReturnType<typeof startWebsite>> | undefined;
  t.after(async () => {
    await server?.close();
    removeFixtureRoot(f.root);
  });
  for (const layout of ['doc', 'page', 'full', 'home']) {
    write(path.join(f.site, 'content', `${layout}.md`), `---\nlayout: ${layout}\n---\n${body}`);
  }
  write(path.join(f.site, 'content/index.md'), '---\nlayout: home\n---\n# Home');
  write(path.join(f.site, 'content/enabled.md'), `---\noutline: true\n---\n${body}`);
  write(path.join(f.site, 'content/disabled.md'), `---\noutline: false\n---\n${body}`);
  write(path.join(f.site, 'content/short.md'), '# Short\n\n## Only section\n\nText.');
  write(path.join(f.site, 'theme.css'), '.toc-rail { --docs-color-brand: rgb(0, 103, 184); }');
  const customHtml = '<h1>Custom</h1><h2 id="one">One</h2><h6 id="two&amp;more">Two &amp; more</h6><h2>Unanchored</h2>' +
    '<for id="outline-links" each="item in authoredItems"><span>{{item}}</span></for>';
  write(f.configFile, JSON.stringify({
    ...f.config, basePath: base, outline,
    customPages: {
      '/custom': { html: customHtml, outline: true },
      '/custom-disabled': { html: customHtml, outline: false },
    },
    regions: replacement === undefined ? {} : { 'page.outline': { html: replacement } },
  }));
  build(f.site, `--show=${mode}`);
  const output = path.join(f.site, 'dist');
  server = await startWebsite(output, base);
  return { ...f, output, origin: server.origin };
}

async function loadedScripts(page: Page): Promise<string> {
  const urls = await page.evaluate(() => performance.getEntriesByType('resource')
    .map(entry => entry.name).filter(url => new URL(url).pathname.endsWith('.js')));
  const contents = await Promise.all(urls.map(async url => {
    const response = await page.request.get(url);
    assert.ok(response.ok(), `Missing script ${url}`);
    return response.text();
  }));
  return contents.join('\n');
}

test('Press outlines are opt-in, with per-page overrides and page-scoped assets', async t => {
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const setting of [undefined, false, true]) {
    const f = await outlineSite(t, setting);
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    for (const [route, enabled] of [
      ['doc', setting === true], ['page', setting === true],
      ['enabled', true], ['disabled', false], ['short', false],
      ['home', false], ['full', false], ['custom', true], ['custom-disabled', false],
    ] as const) {
      const response = await page.goto(`${f.origin}${base}${route}/`);
      await expect(page.locator('docs-outline')).toHaveCount(enabled ? 1 : 0);
      await expect(page.locator('.toc-rail')).toHaveCount(enabled ? 1 : 0);
      await expect(page.locator('link[href$="docs-outline.css"]')).toHaveCount(enabled ? 1 : 0);
      assert.equal((await loadedScripts(page)).includes('docs-outline'), enabled,
        `${String(setting)} / ${route}: component code must follow the outline, not the site shell.`);
      assert.ok(response);
      const html = await response.text();
      const bootstrap = html.match(/<script\b[^>]*\bid="webui-data"[^>]*>([\s\S]*?)<\/script>/)?.[1];
      assert.ok(bootstrap);
      const data = JSON.parse(bootstrap);
      const state = data.state;
      assert.ok(!('page' in state), 'The static outline must not hydrate the page heading inventory.');
      assert.ok(!('headings' in state));
      assert.ok(!JSON.stringify(data).includes('section-36'), 'Heading values must remain server-only.');
      if (enabled) {
        assert.equal(await page.locator('docs-outline').evaluate(element => element.shadowRoot), null,
          'Document anchor tracking needs Light DOM.');
        await expect(page.locator('.outline-desktop a')).toHaveCount(route === 'custom' ? 2 : 36);
      }
    }
    await page.goto(`${f.origin}${base}404.html`);
    await expect(page.locator('.toc-rail, link[href$="docs-outline.css"]')).toHaveCount(0);
    assert.deepEqual(errors, []);
    await page.close();
  }
});

test('the shared outline renders safe anchored links and a native mobile disclosure without JavaScript', async t => {
  const f = await outlineSite(t, true);
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 1440, height: 900 } });
  await page.goto(`${f.origin}${base}custom/`);
  const links = page.locator('.outline-desktop a');
  await expect(links).toHaveText(['One', 'Two & more']);
  await expect(links.last()).toHaveAttribute('href', `${base}custom/#two&more`);
  await expect(links.locator('em, script')).toHaveCount(0);
  await links.last().click();
  assert.equal(new URL(page.url()).hash, '#two&more');

  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${f.origin}${base}doc/`);
  await expect(page.locator('.toc-rail')).toHaveCSS('position', 'static');
  const mobile = page.locator('.outline-mobile');
  await mobile.locator('summary').click();
  await expect(mobile.locator('a')).toHaveCount(36);
  await expect(mobile.locator('nav')).toHaveCSS('overflow-y', 'visible');
  await mobile.locator('a').last().click();
  await expect(page).toHaveURL(/#section-36$/);
});

test('the shared desktop outline is sticky, themed, and follows native section selection', async t => {
  const f = await outlineSite(t, true);
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, reducedMotion: 'reduce' });
  await page.goto(`${f.origin}${base}doc/`);
  await expect(page.locator('.toc-rail')).toHaveCSS('position', 'sticky');
  const outline = page.locator('.outline-desktop ol');
  for (const id of ['section-1', 'section-18', 'section-36']) {
    await page.evaluate(target => {
      const heading = document.getElementById(target);
      if (!heading) throw new Error(`Missing heading ${target}`);
      heading.scrollIntoView({ behavior: 'instant', block: 'start' });
    }, id);
    const readingPosition = await page.locator('.main-content').evaluate(element => element.scrollTop);
    const current = outline.locator('a:target-current');
    await expect(current).toHaveAttribute('href', `${base}doc/#${id}`);
    await expect(current).toHaveAttribute('aria-current', 'location');
    await expect(current).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
    await expect(current).toHaveCSS('text-decoration-line', 'underline');
    await expect(current).toHaveCSS('color', 'rgb(0, 103, 184)');
    await expect.poll(() => current.evaluate(element => {
      const viewport = element.closest('ol');
      if (!viewport) throw new Error('Missing outline viewport');
      const bounds = viewport.getBoundingClientRect();
      const link = element.getBoundingClientRect();
      return link.top >= bounds.top - 1 && link.bottom <= bounds.bottom + 1;
    })).toBe(true);
    assert.equal(await page.locator('.main-content').evaluate(element => element.scrollTop), readingPosition);
  }
  await expect(outline).toHaveCSS('scrollbar-width', 'none');
  await expect(outline).toHaveCSS('scroll-behavior', 'auto');
});

test('the outline component cleans up listeners, survives DOM moves, and supports explicit-state remounts', async t => {
  const f = await outlineSite(t, true);
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, reducedMotion: 'reduce' });
  await page.addInitScript(() => {
    window.testOutlineSignals = [];
    const add = EventTarget.prototype.addEventListener;
    EventTarget.prototype.addEventListener = function (
      this: EventTarget,
      type: string,
      listener: EventListenerOrEventListenerObject | null,
      options?: boolean | AddEventListenerOptions,
    ): void {
      if (type === 'focusout' && this instanceof Element && this.closest('docs-outline') &&
        typeof options === 'object' && options.signal) {
        window.testOutlineSignals.push(options.signal);
      }
      add.call(this, type, listener, options);
    };
  });
  await page.goto(`${f.origin}${base}doc/`);
  await expect(page.locator('.outline-desktop a:target-current')).toHaveAttribute('aria-current', 'location');
  const host = await page.locator('docs-outline').elementHandle();
  assert.ok(host);
  await host.evaluate(element => {
    const main = element.parentElement;
    if (!main) throw new Error('Missing main content');
    element.remove();
    main.prepend(element);
  });
  await expect(page.locator('.outline-desktop a:target-current')).toHaveAttribute('aria-current', 'location');
  await expect(page.locator('.outline-desktop a')).toHaveCount(36);
  assert.deepEqual(await page.evaluate(() => ({
    total: window.testOutlineSignals.length,
    active: window.testOutlineSignals.filter(signal => !signal.aborted).length,
  })), { total: 2, active: 1 });

  for (const [index, id] of ['section-18', 'section-36'].entries()) {
    await host.evaluate(element => element.remove());
    assert.equal(await host.evaluate(element => element.querySelectorAll('[aria-current]').length), 0);
    assert.equal(await page.evaluate(() => window.testOutlineSignals.every(signal => signal.aborted)), true);
    await page.evaluate(target => {
      const heading = document.getElementById(target);
      if (!heading) throw new Error(`Missing heading ${target}`);
      heading.scrollIntoView({ behavior: 'instant', block: 'start' });
    }, id);
    await host.evaluate((element, path) => {
      const main = document.querySelector('.main-content');
      if (!main) throw new Error('Missing main content');
      if (!('setState' in element) || typeof element.setState !== 'function') {
        throw new Error('Missing component state API');
      }
      element.setState({
        path,
        headings: Array.from({ length: 36 }, (_, index) => ({
          text: `Section ${index + 1}`, anchor: `section-${index + 1}`, level: 2,
        })),
      });
      main.prepend(element);
    }, `${base}doc/`);
    const current = page.locator('.outline-desktop a:target-current');
    await expect(current).toHaveAttribute('aria-current', 'location');
    await expect(page.locator('.outline-desktop a')).toHaveCount(36);
    assert.deepEqual(await page.evaluate(() => ({
      total: window.testOutlineSignals.length,
      active: window.testOutlineSignals.filter(signal => !signal.aborted).length,
    })), { total: index + 3, active: 1 });
  }
});

test('content-only output and a replaced outline region omit built-in outline assets', async t => {
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const [mode, replacement] of [['content', undefined], ['all', '<aside>My own outline</aside>']] as const) {
    const f = await outlineSite(t, true, mode, replacement);
    const page = await browser.newPage();
    await page.goto(`${f.origin}${base}doc/`, { waitUntil: 'networkidle' });
    await expect(page.locator('.toc-rail, link[href$="docs-outline.css"]')).toHaveCount(0);
    assert.equal(fs.existsSync(path.join(f.output, 'docs-outline.css')), false);
    assert.equal((await loadedScripts(page)).includes('docs-outline'), false);
    if (replacement) await expect(page.getByText('My own outline')).toBeVisible();
    await page.close();
  }
});

test('invalid outline frontmatter fails instead of silently disabling the feature', async t => {
  const f = fixture();
  t.after(() => removeFixtureRoot(f.root));
  write(f.configFile, JSON.stringify({ ...f.config, regions: {}, outline: true }));
  write(path.join(f.site, 'content/doc.md'), '---\noutline: "false"\n---\n# Invalid');
  assert.throws(() => build(f.site), /outline must be a boolean[\s\S]*help:/);
});
