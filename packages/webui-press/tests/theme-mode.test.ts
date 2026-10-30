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
import { contrastRatio, openAppearance, setColorMode } from './website-fixture.js';

async function themedSite(t: TestContext, mode: 'all' | 'content', families = false) {
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
  const tokens = {
    themes: {
      light: { ...light, 'fixture-text': '#111111', 'fixture-surface': '#ffffff' },
      dark: { ...dark, 'fixture-text': '#eeeeee', 'fixture-surface': '#171717' },
    },
  };
  if (families) {
    Object.assign(tokens.themes.light, { 'docs-color-bg': '#fafafa' });
    Object.assign(tokens.themes.dark, { 'docs-color-bg': '#181818' });
    write(path.join(f.site, '.webui-press/ocean.json'), JSON.stringify({
      themes: {
        light: { ...tokens.themes.light, 'docs-color-bg': '#f0f8ff' },
        dark: { ...tokens.themes.dark, 'docs-color-bg': '#001a33' },
      },
    }));
  }
  write(path.join(f.site, '.webui-press/tokens.json'), JSON.stringify(tokens));
  write(path.join(f.site, 'theme.css'), `
    :root { --docs-nav-height: 4.5rem; }
    @media (forced-colors: active) {
      :root { --fixture-text: CanvasText; --fixture-surface: Canvas; }
    }
  `);
  write(f.configFile, JSON.stringify({ ...f.config, regions: {}, ...(families ? {
    appearance: {
      defaultTheme: 'neutral',
      themes: [
        { id: 'neutral', label: 'Neutral', source: './tokens.json' },
        { id: 'ocean', label: 'Ocean', source: './ocean.json' },
      ],
    },
  } : { theme: './tokens.json' }) }));
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
  const contrast = contrastRatio(colors.color, colors.background);
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
  await openAppearance(page);
  await expect(page.getByRole('group', { name: 'Color mode' }).getByRole('button', { name: 'System' })).toHaveAttribute('aria-pressed', 'true');
  await setColorMode(page, 'light');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await expectTheme(page, 'light');
  await expect(page.getByRole('group', { name: 'Color mode' }).getByRole('button', { name: 'Light', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('theme-probe input')).toHaveCSS('color-scheme', 'light');
  await page.reload();
  await expectTheme(page, 'light');
  await capture(page, 'theme-full-light-os-dark');
  await expectForcedColors(page);
  await page.emulateMedia({ forcedColors: 'none', colorScheme: 'light' });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await setColorMode(page, 'dark');
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

test('sourced family colors override shell defaults while authored aliases retain precedence', async t => {
  const { url } = await themedSite(t, 'all', true);
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const page = await browser.newPage({ colorScheme: 'dark', reducedMotion: 'reduce' });
  await page.goto(url);
  await expect(page.locator('body')).toHaveCSS('background-color', 'rgb(24, 24, 24)');
  const family = page.getByRole('combobox', { name: 'Design theme' });
  await openAppearance(page);
  for (const [id, light, dark] of [
    ['neutral', 'rgb(250, 250, 250)', 'rgb(24, 24, 24)'],
    ['ocean', 'rgb(240, 248, 255)', 'rgb(0, 26, 51)'],
  ]) {
    await family.selectOption(id);
    for (const preference of ['system', 'light', 'dark'] as const) {
      await setColorMode(page, preference);
      for (const colorScheme of ['light', 'dark'] as const) {
        await page.emulateMedia({ colorScheme });
        const resolved = preference === 'system' ? colorScheme : preference;
        await expect(page.locator('body')).toHaveCSS('background-color', resolved === 'light' ? light : dark);
        assert.equal(await page.evaluate(() =>
          getComputedStyle(document.documentElement).getPropertyValue('--docs-nav-height').trim()), '4.5rem');
      }
    }
  }
  await expectForcedColors(page);
  const noJs = await browser.newPage({ javaScriptEnabled: false, colorScheme: 'dark' });
  await noJs.goto(url);
  await expect(noJs.locator('body')).toHaveCSS('background-color', 'rgb(24, 24, 24)');
  await noJs.emulateMedia({ colorScheme: 'light' });
  await expect(noJs.locator('body')).toHaveCSS('background-color', 'rgb(250, 250, 250)');
});
