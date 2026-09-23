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

async function themedSite(t: TestContext, mode: 'all' | 'content') {
  const f = fixture();
  const root = path.join(f.site, 'dist');
  let server: http.Server | undefined;
  t.after(async () => {
    const running = server;
    if (running) {
      running.closeAllConnections();
      await new Promise<void>(resolve => running.close(() => resolve()));
    }
    removeFixtureRoot(f.root);
  });
  write(path.join(f.site, 'content/local/theme-probe/theme-probe.html'),
    '<p class="probe">Readable native theme tokens</p><label>Native field <input value="System-aware control"></label>');
  write(path.join(f.site, 'content/local/theme-probe/theme-probe.css'), `
    :host { display: block; }
    .probe { color: var(--fixture-text); background: var(--docs-color-bg, #fff); padding: 16px; }
    input { color: var(--fixture-text); background: var(--fixture-surface); font: inherit; max-width: 100%; }
  `);
  write(path.join(f.site, 'content/theme.md'), '# Theme fixture\n\n<theme-probe></theme-probe>');
  const css = fs.readFileSync(
    path.resolve(import.meta.dirname, '../../../crates/webui-press/template/docs.css'),
    'utf8',
  );
  const darkStart = css.indexOf('[data-theme="dark"]');
  assert.ok(darkStart > 0);
  const declarations = (source: string) => Object.fromEntries(
    Array.from(source.matchAll(/--([\w-]+):\s*([^;]+);/g), match => [match[1], match[2].trim()]),
  );
  // Reuse Press's own palette; the fixture authors only its component tokens.
  const light = declarations(css.slice(0, darkStart));
  const dark = { ...light, ...declarations(css.slice(darkStart)) };
  write(path.join(f.site, '.webui-press/tokens.json'), JSON.stringify({
    themes: {
      light: { ...light, 'fixture-text': '#111111', 'fixture-surface': '#ffffff' },
      dark: { ...dark, 'fixture-text': '#eeeeee', 'fixture-surface': '#171717' },
    },
  }));
  write(path.join(f.site, 'theme.css'), `
    @media (forced-colors: active) {
      :root { --fixture-text: CanvasText; --fixture-surface: Canvas; }
    }
  `);
  write(f.configFile, JSON.stringify({ ...f.config, regions: {}, theme: './tokens.json' }));
  build(f.site, `--show=${mode}`);
  const mime: Record<string, string> = {
    '.css': 'text/css', '.js': 'text/javascript', '.json': 'application/json', '.svg': 'image/svg+xml',
  };
  server = http.createServer((request, response) => {
    const url = new URL(request.url ?? '/', 'http://localhost');
    const relative = decodeURIComponent(url.pathname).replace(/^\/fixture\/?/, '');
    let file = path.resolve(root, relative);
    if (!file.startsWith(root + path.sep) && file !== root) {
      response.writeHead(403).end();
      return;
    }
    if (fs.existsSync(file) && fs.statSync(file).isDirectory()) file = path.join(file, 'index.html');
    if (!fs.existsSync(file)) file = path.join(root, '404.html');
    response.setHeader('Content-Type', mime[path.extname(file)] ?? 'text/html');
    fs.createReadStream(file).pipe(response);
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const address = server.address();
  assert.ok(address && typeof address !== 'string');
  return { url: `http://127.0.0.1:${address.port}/fixture/theme/`, root };
}

function luminance(color: string): number {
  const channels = color.match(/\d+/g)?.slice(0, 3).map(Number);
  assert.ok(channels && channels.length === 3);
  const [red, green, blue] = channels.map(value => {
    const channel = value / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

async function expectTheme(page: Page, mode: 'light' | 'dark'): Promise<void> {
  const probe = page.locator('theme-probe .probe');
  await expect(probe).toHaveCSS('color', mode === 'light' ? 'rgb(17, 17, 17)' : 'rgb(238, 238, 238)');
  await expect(page.locator('theme-probe input')).toHaveCSS(
    'background-color', mode === 'light' ? 'rgb(255, 255, 255)' : 'rgb(23, 23, 23)',
  );
  const colors = await probe.evaluate(element => ({
    color: getComputedStyle(element).color,
    background: getComputedStyle(element).backgroundColor,
  }));
  const foreground = luminance(colors.color);
  const background = luminance(colors.background);
  const contrast = (Math.max(foreground, background) + 0.05) / (Math.min(foreground, background) + 0.05);
  assert.ok(contrast >= 4.5, `native token label contrast ${contrast}`);
}

async function expectForcedColors(page: Page): Promise<void> {
  await page.emulateMedia({ forcedColors: 'active' });
  await expect.poll(() => page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue('--fixture-text').trim(),
  )).toBe('CanvasText');
  await expect.poll(() => page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue('--fixture-surface').trim(),
  )).toBe('Canvas');
}

async function capture(page: Page, name: string): Promise<void> {
  const directory = process.env.WEBUI_PRESS_SCREENSHOTS;
  if (!directory) return;
  fs.mkdirSync(directory, { recursive: true });
  for (const viewport of [{ width: 1440, height: 1000 }, { width: 390, height: 844 }]) {
    await page.setViewportSize(viewport);
    await page.screenshot({ path: path.join(directory, `${name}-${viewport.width}.png`), fullPage: true });
  }
}

test('full manual themes override OS token colors without overriding forced colors', async t => {
  const { url } = await themedSite(t, 'all');
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ colorScheme: 'dark' });
  await page.goto(url);
  await expect.poll(() => page.locator('.logo img').evaluate(image =>
    image instanceof HTMLImageElement && image.complete && image.naturalWidth > 0,
  )).toBe(true);
  await expectTheme(page, 'dark');
  await page.locator('docs-theme-toggle button').click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await expectTheme(page, 'light');
  await expect(page.locator('theme-probe input')).toHaveCSS('color-scheme', 'light');
  await page.reload();
  await expectTheme(page, 'light');
  await capture(page, 'theme-full-light-os-dark');
  await expectForcedColors(page);
  await page.emulateMedia({ forcedColors: 'none', colorScheme: 'light' });
  await page.locator('docs-theme-toggle button').click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expectTheme(page, 'dark');
  await expectForcedColors(page);
});

test('content themes ignore stored shell preferences and follow OS changes', async t => {
  const { url, root } = await themedSite(t, 'content');
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({
    colorScheme: 'light',
    storageState: { cookies: [], origins: [{
      origin: new URL(url).origin, localStorage: [{ name: 'theme', value: 'dark' }],
    }] },
  });
  await page.goto(url);
  await expectTheme(page, 'light');
  await expect(page.locator('html')).not.toHaveAttribute('data-theme');
  assert.equal(await page.evaluate(() => localStorage.getItem('theme')), 'dark');
  assert.ok(!fs.readFileSync(path.join(root, 'theme/index.html'), 'utf8').includes('localStorage'));
  await page.emulateMedia({ colorScheme: 'dark' });
  await expectTheme(page, 'dark');
  await capture(page, 'theme-content-system-dark');
  await page.emulateMedia({ colorScheme: 'light' });
  await expectTheme(page, 'light');
  await capture(page, 'theme-content-system-light');
  await expectForcedColors(page);

  const noJs = await browser.newPage({ javaScriptEnabled: false, colorScheme: 'dark' });
  await noJs.goto(url);
  await expectTheme(noJs, 'dark');
  await noJs.emulateMedia({ colorScheme: 'light' });
  await expectTheme(noJs, 'light');
});
