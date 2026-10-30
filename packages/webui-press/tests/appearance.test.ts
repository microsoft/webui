// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import type { Browser, Page } from '@playwright/test';
import { buildSync } from 'esbuild';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import type { TestContext } from 'node:test';
import { build, fixture, removeFixtureRoot, write } from './native-fixture.js';
import { openAppearance, setColorMode, startWebsite } from './website-fixture.js';
import type * as AppearanceAPI from '../../../crates/webui-press/template/appearance.js';

declare global {
  interface Window {
    press: typeof AppearanceAPI;
    updates: AppearanceAPI.Appearance[];
    stopAppearance: () => void;
    transitionCount: number;
    themeMutations: number;
    appearancePaints: { name: string; theme: string | null; mode: string | null }[];
  }
}

async function inlineScriptHashes(browser: Browser, html: string, bootstrap: string): Promise<string[]> {
  const page = await browser.newPage({ javaScriptEnabled: false });
  try {
    await page.route('**/*', route => route.abort());
    await page.setContent(html, { waitUntil: 'domcontentloaded' });
    const scripts = await page.evaluate(() => Array.from(document.scripts)
      .filter(script => !script.hasAttribute('src') && script.type.trim().toLowerCase() !== 'application/json')
      .map(script => script.textContent ?? ''));
    return scripts.filter(script => script.trim() !== bootstrap)
      .map(script => `'sha256-${createHash('sha256').update(script).digest('base64')}'`);
  } finally {
    await page.close();
  }
}

async function appearanceSite(t: TestContext, options: { customHeader?: boolean; csp?: Browser } = {}) {
  const { customHeader = false, csp } = options;
  const f = fixture();
  t.after(() => removeFixtureRoot(f.root));
  const probe = path.join(f.site, 'content/local/appearance-probe/appearance-probe');
  write(`${probe}.html`, '<span>{{appearance}}</span>');
  write(`${probe}.ts`, `
import { WebUIElement, attr } from '@microsoft/webui-framework';
class AppearanceProbe extends WebUIElement {
  @attr appearance = '';
}
AppearanceProbe.define('appearance-probe');
`);
  write(path.join(f.site, 'content/index.md'), `# Appearance

<appearance-probe></appearance-probe>
<button type="button">Outside action</button>

${customHeader ? '' : `<script type="module" bundle>
import * as press from '@microsoft/webui-press/appearance.js';
window.press = press;
window.updates = [];
window.stopAppearance = press.subscribeAppearance(value => window.updates.push(value));
</script>`}
`);
  write(f.configFile, JSON.stringify({
    ...f.config, regions: customHeader ? {
      'site.navigation': { html: '<header>Custom navigation without controls</header>' },
    } : {},
    appearance: {
      themes: [{ id: 'neutral', label: 'Neutral' }, { id: 'colorful', label: 'Colorful' }],
      defaultTheme: 'neutral', defaultMode: 'system',
      storage: { theme: 'fixture-family', mode: 'fixture-mode' },
    },
  }));
  build(f.site);
  const root = path.join(f.site, 'dist');
  const headers: Record<string, string> = {};
  if (csp) {
    const html = fs.readFileSync(path.join(root, 'index.html'), 'utf8');
    const bootstrap = fs.readFileSync(
      path.resolve(import.meta.dirname, '../../../crates/webui-press/template/appearance-bootstrap.js'), 'utf8',
    ).trim();
    const hashes = await inlineScriptHashes(csp, html, bootstrap);
    headers['content-security-policy'] = `script-src 'self' ${hashes.join(' ')}`;
  }
  const server = await startWebsite(root, '/fixture/', headers);
  t.after(() => server.close());
  return { ...f, dist: root, url: `${server.origin}/fixture/` };
}

async function ready(page: Page, url: string): Promise<void> {
  await page.goto(url);
  await page.waitForFunction(() => !!window.press);
  await openAppearance(page);
  await expect(page.getByRole('group', { name: 'Color mode' }).getByRole('button', { name: 'Light', exact: true })).toBeEnabled();
}

async function blockHydrationScripts(page: Page, url: string): Promise<void> {
  const origin = new URL(url).origin;
  await page.route('**/*', route => {
    const request = route.request();
    const source = new URL(request.url());
    if (request.resourceType() !== 'script' ||
      (source.origin === origin && /\/appearance-init-[a-f0-9]+\.js$/.test(source.pathname))) {
      return route.continue();
    }
    return route.abort();
  });
}

test('full-shell sourced families validate bundled and authored CSS without changing snapshots', t => {
  const f = fixture();
  t.after(() => removeFixtureRoot(f.root));
  const page = path.join(f.site, 'content/index.md');
  write(page, '# Sourced family');
  write(path.join(f.site, '.webui-press/tokens.json'), JSON.stringify({
    themes: { light: { surface: '#fff' }, dark: { surface: '#000' } },
  }));
  write(path.join(f.site, 'theme.css'), ':root { --consumer-surface: var(--surface); }');
  write(f.configFile, JSON.stringify({
    ...f.config, regions: {},
    appearance: {
      defaultTheme: 'official',
      themes: [{ id: 'official', label: 'Official', source: './tokens.json' }],
    },
  }));
  build(f.site);
  const output = path.join(f.site, 'dist');
  const css = fs.readFileSync(path.join(output, 'appearance.css'), 'utf8');
  assert.ok(!css.includes('--docs-color-bg-alt:'));
  assert.ok(!css.includes('--consumer-surface:'));
  assert.ok(css.includes('--surface: #fff;'));
  assert.ok(fs.existsSync(path.join(output, '404.html')));

  write(page, '# Consumer CSS\n\n<style>.sample { color: var(--consumer-surface); }</style>');
  build(f.site);
  write(page, '# Typo\n\n<style>.sample { color: var(--consumer-surfac); }</style>');
  assert.throws(() => build(f.site), error => {
    assert.ok(error instanceof Error);
    assert.match(error.message, /consumer-surfac/);
    assert.match(error.message, /help:/);
    return true;
  });
});

test('CSP fixture hashes browser-parsed inline scripts without executing them', async t => {
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const scripts = [
    'document.querySelectorAll("script").forEach(script => script.remove());',
    'window.second = true;',
    'window.third = true;',
  ];
  const bootstrap = 'window.bootstrap = true;';
  const html = `<!doctype html>
<script data-label=">">${scripts[0]}</script >
<script data-src="/not-external.js">${scripts[1]}</SCRIPT\t>
<script>${scripts[2]}</script
>
<script src="/external.js">window.external = true;</script>
<script type=" application/JSON ">{"inert":true}</script>
<script>${bootstrap}</script>`;
  assert.deepEqual(await inlineScriptHashes(browser, html, bootstrap),
    scripts.map(script => `'sha256-${createHash('sha256').update(script).digest('base64')}'`));
});

test('same-origin appearance initialization works under CSP without authorizing inline bootstrap', async t => {
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const f = await appearanceSite(t, { csp: browser });
  const context = await browser.newContext({
    colorScheme: 'light', reducedMotion: 'reduce', storageState: {
      cookies: [], origins: [{ origin: new URL(f.url).origin, localStorage: [
        { name: 'fixture-family', value: 'colorful' }, { name: 'fixture-mode', value: 'dark' },
      ] }],
    },
  });
  const page = await context.newPage();
  const errors: string[] = [];
  const initializationRequests: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  page.on('request', request => {
    if (/\/appearance-init-[a-f0-9]+\.js$/.test(new URL(request.url()).pathname)) {
      initializationRequests.push(request.url());
    }
  });
  await ready(page, f.url);
  assert.equal(initializationRequests.length, 1);
  assert.equal(await page.locator('script[src*="appearance-init-"]').evaluate(script =>
    script instanceof HTMLScriptElement && !script.async && !script.defer), true);
  const family = page.getByRole('combobox', { name: 'Design theme', exact: true });
  await expect(family).toHaveValue('colorful');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await family.selectOption('neutral');
  await setColorMode(page, 'light');
  await page.reload();
  await openAppearance(page);
  await expect(family).toHaveValue('neutral');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await setColorMode(page, 'system');
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  assert.deepEqual(errors, []);

  const prepaint = await browser.newPage({
    colorScheme: 'light', storageState: {
      cookies: [], origins: [{ origin: new URL(f.url).origin, localStorage: [
        { name: 'fixture-family', value: 'colorful' }, { name: 'fixture-mode', value: 'dark' },
      ] }],
    },
  });
  await prepaint.addInitScript(() => {
    window.appearancePaints = [];
    new PerformanceObserver(list => {
      for (const entry of list.getEntries()) {
        window.appearancePaints.push({
          name: entry.name,
          theme: document.documentElement.getAttribute('data-press-theme'),
          mode: document.documentElement.getAttribute('data-theme'),
        });
      }
    }).observe({ type: 'paint', buffered: true });
  });
  await blockHydrationScripts(prepaint, f.url);
  await prepaint.route('**/appearance-init-*.js', async route => {
    await new Promise(resolve => setTimeout(resolve, 150));
    await route.continue();
  });
  await prepaint.goto(f.url);
  await expect(prepaint.locator('html')).toHaveAttribute('data-press-theme', 'colorful');
  await expect(prepaint.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(prepaint.getByRole('button', { name: 'Appearance', exact: true })).toBeDisabled();
  await expect.poll(() => prepaint.evaluate(() =>
    window.appearancePaints.some(entry => entry.name === 'first-contentful-paint'))).toBe(true);
  for (const paint of await prepaint.evaluate(() => window.appearancePaints)) {
    assert.equal(paint.theme, 'colorful', `${paint.name}: family must be restored before paint`);
    assert.equal(paint.mode, 'dark', `${paint.name}: mode must be restored before paint`);
  }

  const unavailable = await browser.newPage();
  const failures: string[] = [];
  unavailable.on('pageerror', error => failures.push(error.message));
  await unavailable.route('**/appearance-init-*.js', route => route.abort());
  await unavailable.goto(f.url);
  await expect.poll(() => failures.some(error => error.includes('Deploy the generated appearance-init script, allow it in your CSP'))).toBe(true);
});

test('shared appearance controls, prepaint restore, cross-tab state, OS changes and packed API', async t => {
  const f = await appearanceSite(t);
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const context = await browser.newContext({ colorScheme: 'dark', reducedMotion: 'reduce' });
  const page = await context.newPage();
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await ready(page, f.url);
  await page.evaluate(() => customElements.whenDefined('appearance-probe'));
  assert.equal(await page.locator('appearance-probe').evaluate(element =>
    Reflect.get(element, 'appearance')), '');
  const family = page.getByRole('combobox', { name: 'Design theme', exact: true });
  const mode = page.getByRole('group', { name: 'Color mode' });
  const palette = page.getByRole('button', { name: 'Appearance', exact: true });
  await page.keyboard.press('Escape');
  await expect(palette).toBeFocused();
  await expect(family).not.toBeVisible();
  await palette.press('Enter');
  await expect(family).toBeVisible();
  const outside = page.getByRole('button', { name: 'Outside action' });
  await outside.click();
  await expect(family).not.toBeVisible();
  await expect(outside).toBeFocused();
  await palette.click();
  await expect(family).toHaveValue('neutral');
  await expect(mode.getByRole('button', { name: 'System' })).toHaveAttribute('aria-pressed', 'true');
  await expect(mode.getByRole('button', { name: 'Light', exact: true })).toHaveAttribute('aria-pressed', 'false');
  await expect(mode.getByRole('button', { name: 'Dark', exact: true })).toHaveAttribute('aria-pressed', 'false');
  await expect(page.getByRole('combobox', { name: 'Color mode' })).toHaveCount(0);
  assert.deepEqual(await page.evaluate(() => window.press.getAppearance()), {
    theme: 'neutral', mode: 'system', resolvedMode: 'dark',
  });
  assert.equal(await page.evaluate(() => Object.isFrozen(window.press.getThemeFamilies())), true);
  assert.equal(await page.evaluate(() => window.updates.length), 0);
  await family.selectOption('colorful');
  await page.evaluate(() => window.press.setAppearance({ theme: 'colorful' }));
  assert.equal(await page.evaluate(() => window.updates.length), 1);
  await setColorMode(page, 'light');
  await expect(page.locator('html')).toHaveAttribute('data-press-theme', 'colorful');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.reload();
  await openAppearance(page);
  await expect(mode.getByRole('button', { name: 'Light', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(family).toHaveValue('colorful');
  await setColorMode(page, 'system');
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');

  const second = await context.newPage();
  await ready(second, f.url);
  await second.evaluate(() => window.press.setAppearance({ theme: 'neutral', mode: 'light' }));
  await expect(family).toHaveValue('neutral');
  await expect(mode.getByRole('button', { name: 'Light', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.evaluate(() => {
    window.stopAppearance();
    window.updates = [];
  });
  await setColorMode(page, 'dark');
  assert.equal(await page.evaluate(() => window.updates.length), 0);
  const before = await page.evaluate(() => window.press.getAppearance());
  const invalid = await page.evaluate(async () => {
    try { await window.press.setAppearance({ theme: 'missing' }); return ''; }
    catch (error) { return String(error); }
  });
  assert.match(invalid, /TypeError/);
  assert.deepEqual(await page.evaluate(() => window.press.getAppearance()), before);
  await setColorMode(page, 'light');
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.keyboard.press('Escape');
  await expect(family).not.toBeVisible();
  await expect(palette).toBeFocused();
  await palette.click();
  await setColorMode(page, 'dark');

  // Exercise the real published export, not a workspace-relative source import.
  const packageDir = path.resolve(import.meta.dirname, '..');
  const packed = JSON.parse(execFileSync('npm', ['pack', '--json', '--pack-destination', f.root], {
    cwd: packageDir, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
  })) as { filename: string }[];
  execFileSync('tar', ['-xzf', path.join(f.root, packed[0].filename), '-C', f.root]);
  const pkg = JSON.parse(fs.readFileSync(path.join(f.root, 'package/package.json'), 'utf8'));
  assert.equal(pkg.exports['./appearance.js'].types, './dist/appearance.d.ts');
  assert.match(fs.readFileSync(path.join(f.root, 'package/dist/appearance.d.ts'), 'utf8'), /subscribeAppearance/);
  const packageLink = path.join(f.root, 'node_modules/@microsoft/webui-press');
  fs.mkdirSync(path.dirname(packageLink), { recursive: true });
  fs.symlinkSync(path.join(f.root, 'package'), packageLink, 'junction');
  execFileSync(process.execPath, ['--input-type=module', '-e',
    `import {getAppearance,getThemeFamilies,setAppearance,subscribeAppearance} from '@microsoft/webui-press/appearance.js';
     if (![getAppearance,getThemeFamilies,setAppearance,subscribeAppearance].every(fn => typeof fn === 'function')) process.exit(1);`,
  ], { cwd: f.root });
  write(path.join(f.root, 'consumer.mts'), `
    import {getAppearance, getThemeFamilies, setAppearance, subscribeAppearance} from '@microsoft/webui-press/appearance.js';
    const mode: 'system' | 'light' | 'dark' = getAppearance().mode;
    const families: readonly {id: string; label: string}[] = getThemeFamilies();
    const update: Promise<void> = setAppearance({theme: families[0].id, mode});
    const stop: () => void = subscribeAppearance(value => console.log(value.resolvedMode));
    // @ts-expect-error invalid appearance is rejected by the public types
    setAppearance({mode: 'automatic'});
  `);
  write(path.join(f.root, 'tsconfig.json'), JSON.stringify({
    compilerOptions: { noEmit: true, strict: true, skipLibCheck: true, module: 'NodeNext', target: 'ES2022' },
    files: ['consumer.mts'],
  }));
  execFileSync(process.execPath, [
    path.join(packageDir, 'node_modules/typescript/bin/tsc'), '--project', path.join(f.root, 'tsconfig.json'),
  ], { cwd: f.root, stdio: 'pipe' });
  fs.copyFileSync(path.join(f.root, 'package/dist/appearance.js'), path.join(f.dist, 'packed-appearance.js'));
  assert.equal(await page.evaluate(async () => {
    const url = new URL('packed-appearance.js', location.href).href;
    const packed = await import(url);
    return packed.getAppearance() === window.press.getAppearance();
  }), true);
  const screenshotDir = process.env.WEBUI_PRESS_SCREENSHOTS;
  if (screenshotDir) {
    fs.mkdirSync(screenshotDir, { recursive: true });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.screenshot({ path: path.join(screenshotDir, 'appearance-1440.png'), fullPage: true });
  }
  await page.setViewportSize({ width: 390, height: 900 });
  await expect(family).not.toBeVisible();
  if (screenshotDir) {
    await page.screenshot({ path: path.join(screenshotDir, 'appearance-390-closed.png'), fullPage: true });
  }
  const navigation = page.getByRole('button', { name: 'Open navigation', exact: true });
  await navigation.click();
  await expect(family).toBeVisible();
  await expect(mode).toBeVisible();
  // Keyboard selection stays native; Escape closes the picker, then the menu.
  await page.keyboard.press('Tab');
  await expect(family).toBeFocused();
  await page.keyboard.press('Space');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await expect(family).toHaveValue('colorful');
  await page.keyboard.press('Tab');
  const system = mode.getByRole('button', { name: 'System' });
  await expect(system).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(system).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('Tab');
  const light = mode.getByRole('button', { name: 'Light', exact: true });
  await expect(light).toBeFocused();
  await page.keyboard.press('Space');
  await expect(light).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('Tab');
  const dark = mode.getByRole('button', { name: 'Dark', exact: true });
  await expect(dark).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(dark).toHaveAttribute('aria-pressed', 'true');
  if (screenshotDir) {
    await page.screenshot({ path: path.join(screenshotDir, 'appearance-390.png'), fullPage: true });
  }
  await page.keyboard.press('Escape');
  await expect(navigation).toBeFocused();
  await expect(family).not.toBeVisible();
  assert.deepEqual(errors, []);
});

test('bootstrap validates stored choices, preserves legacy keys and works without storage or hydration', async t => {
  const f = await appearanceSite(t);
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const page = await browser.newPage({ colorScheme: 'light', reducedMotion: 'reduce' });
  await page.addInitScript(() => {
    localStorage.setItem('theme', 'dark');
    localStorage.setItem('fixture-family', 'missing');
    localStorage.removeItem('fixture-mode');
  });
  await ready(page, f.url);
  assert.deepEqual(await page.evaluate(() => window.press.getAppearance()), {
    theme: 'neutral', mode: 'dark', resolvedMode: 'dark',
  });
  await page.evaluate(() => window.press.setAppearance({ mode: 'system' }));
  assert.equal(await page.evaluate(() => localStorage.getItem('theme')), 'dark');
  const prepaint = await browser.newPage({ colorScheme: 'light', storageState: {
    cookies: [], origins: [{ origin: new URL(f.url).origin, localStorage: [
      { name: 'fixture-family', value: 'colorful' }, { name: 'fixture-mode', value: 'dark' },
    ] }],
  } });
  await blockHydrationScripts(prepaint, f.url);
  await prepaint.goto(f.url);
  await expect(prepaint.locator('html')).toHaveAttribute('data-press-theme', 'colorful');
  await expect(prepaint.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(prepaint.getByRole('button', { name: 'Appearance', exact: true })).toBeDisabled();
  await expect(prepaint.getByRole('button', { name: 'Light', exact: true, includeHidden: true })).toBeDisabled();
  const invalidSaved = await browser.newPage({ colorScheme: 'light', storageState: {
    cookies: [], origins: [{ origin: new URL(f.url).origin, localStorage: [
      { name: 'theme', value: 'dark' }, { name: 'fixture-mode', value: 'invalid' },
    ] }],
  } });
  await ready(invalidSaved, f.url);
  assert.deepEqual(await invalidSaved.evaluate(() => window.press.getAppearance()), {
    theme: 'neutral', mode: 'system', resolvedMode: 'light',
  });

  const unavailable = await browser.newPage({ reducedMotion: 'reduce' });
  const warnings: string[] = [];
  unavailable.on('console', message => { if (message.type() === 'warning') warnings.push(message.text()); });
  await unavailable.addInitScript(() => {
    Object.defineProperty(window, 'localStorage', { get() { throw new DOMException('Denied', 'SecurityError'); } });
  });
  await ready(unavailable, f.url);
  await setColorMode(unavailable, 'dark');
  await expect(unavailable.locator('html')).toHaveAttribute('data-theme', 'dark');
  assert.ok(warnings.some(warning => warning.includes('storage is unavailable')));

  const fallback = await browser.newPage({ reducedMotion: 'reduce' });
  await fallback.addInitScript(() => {
    Object.defineProperty(HTMLElement.prototype, 'showPopover', { value: undefined });
  });
  await ready(fallback, f.url);
  await expect(fallback.getByRole('button', { name: 'Appearance', exact: true })).not.toBeVisible();
  await expect(fallback.getByRole('combobox', { name: 'Design theme' })).toBeVisible();
  await setColorMode(fallback, 'dark');
  await expect(fallback.locator('html')).toHaveAttribute('data-theme', 'dark');

  const noJs = await browser.newPage({ javaScriptEnabled: false });
  await noJs.goto(f.url);
  await expect(noJs.getByRole('button', { name: 'Appearance', exact: true })).toBeDisabled();
  await expect(noJs.getByRole('button', { name: 'Light', exact: true, includeHidden: true })).toBeDisabled();
  await expect(noJs.getByRole('combobox', { name: 'Design theme', exact: true, includeHidden: true })).toBeDisabled();
  await expect(noJs.getByRole('combobox', { name: 'Design theme', exact: true, includeHidden: true })).toHaveValue('neutral');
  await noJs.setViewportSize({ width: 390, height: 844 });
  await expect(noJs.getByRole('combobox', { name: 'Design theme', exact: true })).not.toBeVisible();
  assert.equal(await noJs.evaluate(() =>
    document.documentElement.scrollWidth <= window.innerWidth), true);
});

test('native transitions are latest-wins, scoped, skippable and surface unexpected failures', async t => {
  const f = await appearanceSite(t);
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const page = await browser.newPage({ reducedMotion: 'no-preference' });
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await ready(page, f.url);
  await page.evaluate(() => {
    const start = document.startViewTransition.bind(document);
    window.transitionCount = 0;
    document.startViewTransition = ((update: () => void) => {
      window.transitionCount++;
      return start(update);
    }) as typeof document.startViewTransition;
  });
  await page.evaluate(async () => {
    const first = window.press.setAppearance({ theme: 'colorful' });
    await new Promise(resolve => setTimeout(resolve, 10));
    await Promise.all([first, window.press.setAppearance({ mode: 'dark' }),
      window.press.setAppearance({ theme: 'neutral', mode: 'light' })]);
  });
  assert.deepEqual(await page.evaluate(() => window.press.getAppearance()), {
    theme: 'neutral', mode: 'light', resolvedMode: 'light',
  });
  await expect(page.locator('html')).not.toHaveAttribute('data-press-transition');
  assert.ok(await page.evaluate(() => window.transitionCount <= 2));
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const count = await page.evaluate(() => window.transitionCount);
  await page.evaluate(() => window.press.setAppearance({ mode: 'dark' }));
  assert.equal(await page.evaluate(() => window.transitionCount), count);
  await page.emulateMedia({ reducedMotion: 'no-preference', forcedColors: 'active' });
  await page.evaluate(() => window.press.setAppearance({ mode: 'light' }));
  assert.equal(await page.evaluate(() => window.transitionCount), count);
  await page.emulateMedia({ forcedColors: 'none' });
  const failure = await page.evaluate(async () => {
    document.startViewTransition = () => { throw new Error('Transition failure'); };
    try { await window.press.setAppearance({ mode: 'dark' }); return ''; }
    catch (error) { return String(error); }
  });
  assert.match(failure, /Transition failure/);
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(page.locator('html')).not.toHaveAttribute('data-press-transition');
  const rejectedReady = await page.evaluate(async () => {
    document.startViewTransition = ((update: () => void) => {
      update();
      return {
        ready: Promise.reject(new Error('Unexpected ready failure')),
        updateCallbackDone: Promise.resolve(), finished: Promise.resolve(), skipTransition() {},
      };
    }) as typeof document.startViewTransition;
    try { await window.press.setAppearance({ mode: 'light' }); return ''; }
    catch (error) { return String(error); }
  });
  assert.match(rejectedReady, /Unexpected ready failure/);
  await page.evaluate(async () => {
    Object.defineProperty(document, 'startViewTransition', { value: undefined, configurable: true });
    await window.press.setAppearance({ mode: 'dark' });
  });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  assert.deepEqual(errors, []);
});

test('custom headers follow System before the API adopts the bootstrap listener', async t => {
  const f = await appearanceSite(t, { customHeader: true });
  buildSync({
    entryPoints: [path.resolve(import.meta.dirname, '../../../crates/webui-press/template/appearance.ts')],
    outfile: path.join(f.dist, 'late-appearance.js'), format: 'esm', bundle: true,
  });
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const context = await browser.newContext({ colorScheme: 'light', reducedMotion: 'reduce' });
  const page = await context.newPage();
  await page.goto(f.url);
  assert.equal(await page.evaluate(() =>
    Reflect.has(document, Symbol.for('@microsoft/webui-press/appearance'))), false);
  await expect(page.getByRole('combobox')).toHaveCount(0);
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(page.locator('body')).toHaveCSS('background-color', 'rgb(26, 26, 46)');
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  const second = await page.context().newPage();
  await second.goto(f.url);
  await second.evaluate(() => {
    localStorage.setItem('fixture-family', 'colorful');
    localStorage.setItem('fixture-mode', 'dark');
  });
  await expect(page.locator('html')).toHaveAttribute('data-press-theme', 'colorful');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await second.evaluate(() => localStorage.setItem('fixture-mode', 'system'));
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.evaluate(async () => {
    window.press = await import(new URL('late-appearance.js', location.href).href);
    window.updates = [];
    window.stopAppearance = window.press.subscribeAppearance(value => window.updates.push(value));
    window.themeMutations = 0;
    new MutationObserver(records => { window.themeMutations += records.length; })
      .observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  });
  assert.equal(await page.evaluate(() =>
    Reflect.has(document, Symbol.for('@microsoft/webui-press/appearance-bootstrap'))), false);
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  assert.equal(await page.evaluate(() => window.updates.length), 1);
  assert.equal(await page.evaluate(() => window.themeMutations), 1);
  await page.evaluate(() => { window.themeMutations = 0; });
  await second.evaluate(() => localStorage.setItem('fixture-family', 'neutral'));
  await expect(page.locator('html')).toHaveAttribute('data-press-theme', 'neutral');
  assert.equal(await page.evaluate(() => window.updates.length), 2);
  assert.equal(await page.evaluate(() => window.themeMutations), 1);
  await page.evaluate(() => window.press.setAppearance({ mode: 'light' }));
  await page.emulateMedia({ colorScheme: 'light' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
});
