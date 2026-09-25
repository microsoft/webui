// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { Protocol } from '@microsoft/webui';

const require = createRequire(import.meta.url);
const apiOrigin = process.env['CONTACT_BOOK_TEST_API_URL'] ?? 'http://127.0.0.1:3013';
const theme = JSON.parse(
  readFileSync(require.resolve('@microsoft/webui-examples-theme'), 'utf8'),
) as { themes: Record<string, Record<string, string>> };
const tokens = Object.fromEntries(
  Object.entries(theme.themes).map(([name, values]) => [
    name,
    Object.entries(values).map(([key, value]) => `--${key}:${value};`).join(''),
  ]),
);

type WindowBridge = Window & {
  webuiHostPostMessage?: (payload: string) => void;
  windowActions: string[];
};

type TitlebarInsets = { start: number; end: number; height: number };
const windowsInsets: TitlebarInsets = { start: 0, end: 138, height: 48 };

async function captureWindowActions(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const host = window as WindowBridge;
    host.windowActions = [];
    host.webuiHostPostMessage = payload => host.windowActions.push(payload);
  });
}

async function renderDesktopMode(
  page: Page,
  request: APIRequestContext,
  baseURL: string | undefined,
  insets: TitlebarInsets = windowsInsets,
): Promise<void> {
  if (!baseURL) throw new Error('Contact Book tests require a baseURL');
  const origin = new URL(baseURL).origin;
  const protocol = new Protocol(
    readFileSync(new URL('../dist/protocol.bin', import.meta.url)),
    { plugin: 'webui' },
  );

  // Exercise the real compiled templates and Rust renderer with desktop state,
  // retaining the example's real route data, client assets and API server.
  await page.route(`${origin}/**`, async route => {
    const incoming = route.request();
    const url = new URL(incoming.url());
    const document = incoming.resourceType() === 'document';
    const partial = incoming.headers()['accept']?.includes('application/json');
    if (url.pathname.startsWith('/api/') || url.pathname.startsWith('/_webui/')
      || (!document && !partial)) {
      await route.continue();
      return;
    }
    const response = await request.get(
      `${apiOrigin}${url.pathname}${url.search}`,
      { headers: { Accept: 'application/json' } },
    );
    expect(response.ok()).toBe(true);
    const data = await response.json() as { state: Record<string, unknown> };
    const state = { ...data.state, mode: 'desktop', basePath: '/', tokens };
    const body = document
      ? protocol.render(state, { requestPath: url.pathname }).toString('utf8').replace(
        '</head>',
        `<style>:root{--webui-titlebar-inset-start:${insets.start}px;`
          + `--webui-titlebar-inset-end:${insets.end}px;`
          + `--webui-titlebar-height:${insets.height}px}</style></head>`,
      )
      : protocol.renderPartial(
        state, 'index.html', url.pathname, incoming.headers()['x-webui-inventory'] ?? '',
      );
    await route.fulfill({
      status: 200,
      contentType: document ? 'text/html; charset=utf-8' : 'application/json',
      body,
    });
  });
}

async function expectNativeControlSafeAreas(
  page: Page,
  insets: TitlebarInsets = windowsInsets,
): Promise<void> {
  await expect(page.locator('cb-header .window-controls, cb-header .window-control')).toHaveCount(0);
  await expect(page.getByRole('button', { name: /window$/ })).toHaveCount(0);
  await expect(page.locator('cb-header .header')).toHaveAttribute('webui-drag', '');
  const viewport = page.viewportSize();
  if (!viewport) throw new Error('Contact Book tests require a viewport');
  const band = page.locator(viewport.width <= 800 ? 'cb-header .header-left' : 'cb-header .header');
  await expect(band).toHaveCSS('height', `${insets.height}px`);
  await expect(page.locator('cb-header .search-input')).toHaveCSS('height', '38px');
  await expect(page.locator('cb-header .add-btn')).toHaveCSS('height', '40px');
  const title = await page.locator('cb-header .title').boundingBox();
  if (!title) throw new Error('Contact Book title has no layout box');
  expect(title.x).toBeGreaterThanOrEqual(insets.start);
  expect(title.x + title.width).toBeLessThanOrEqual(viewport.width - insets.end);
  expect(title.y + title.height).toBeLessThanOrEqual(insets.height);
  for (const selector of ['.search-container', '.add-btn']) {
    const control = page.locator(`cb-header ${selector}`);
    await expect(control).toBeVisible();
    await expect(control).toHaveAttribute('webui-no-drag', '');
    const rect = await control.boundingBox();
    if (!rect) throw new Error(`${selector} has no hit target`);
    expect(rect.x).toBeGreaterThanOrEqual(0);
    expect(rect.x + rect.width).toBeLessThanOrEqual(viewport.width);
    if (viewport.width <= 800) {
      expect(rect.y).toBeGreaterThanOrEqual(insets.height);
    } else {
      expect(rect.y).toBeGreaterThanOrEqual(0);
      expect(rect.y + rect.height).toBeLessThanOrEqual(insets.height);
      expect(rect.x).toBeGreaterThanOrEqual(insets.start);
      expect(rect.x + rect.width).toBeLessThanOrEqual(viewport.width - insets.end);
    }
  }
}

async function expectViewportContained(page: Page): Promise<void> {
  const size = await page.evaluate(() => ({
    width: innerWidth,
    height: innerHeight,
    scrollWidth: document.documentElement.scrollWidth,
    scrollHeight: document.documentElement.scrollHeight,
  }));
  expect(size.scrollWidth).toBeLessThanOrEqual(size.width + 1);
  expect(size.scrollHeight).toBeLessThanOrEqual(size.height + 1);
  const content = await page.locator('cb-app .content').evaluate(element => ({
    width: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(content.scrollWidth).toBeLessThanOrEqual(content.width + 1);
}

test('web mode omits desktop chrome on initial render and navigation', async ({ page }, testInfo) => {
  await captureWindowActions(page);
  await page.goto('/');
  await expect(page.locator('cb-header')).toHaveAttribute('mode', 'web');
  await expect(page.getByRole('button', { name: /window$/ })).toHaveCount(0);
  await expect(page.locator('[webui-drag]')).toHaveCount(0);
  await page.locator('cb-sidebar').getByRole('link', { name: /All Contacts/ }).click();
  await expect(page.locator('cb-page-contacts .page-title')).toHaveText('All Contacts');
  await expect(page.locator('cb-header')).toHaveAttribute('mode', 'web');
  expect(await page.evaluate(() => (window as WindowBridge).windowActions)).toEqual([]);

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.locator('cb-header .search-input')).toBeVisible();
  await expect(page.locator('cb-header .add-btn')).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth))
    .toBeLessThanOrEqual(390);
  await page.screenshot({ path: testInfo.outputPath('web-narrow.png') });
});

test.describe('desktop mode', () => {
  test.use({ viewport: { width: 1200, height: 800 } });

  test.beforeEach(async ({ page, request, baseURL }) => {
    await renderDesktopMode(page, request, baseURL);
    await captureWindowActions(page);
  });

  test('renders edge to edge and keeps the header fixed during route scrolling', async ({ page }, testInfo) => {
    await page.goto('/');
    await expect(page.locator('cb-header')).toHaveAttribute('mode', 'desktop');
    await expect(page.locator('cb-header')).toHaveJSProperty('$ready', true);
    await expectNativeControlSafeAreas(page);
    const header = await page.locator('cb-header').boundingBox();
    expect(header?.x).toBe(0);
    expect(header?.y).toBe(0);
    expect(header?.width).toBe(1200);
    await expectViewportContained(page);
    await page.screenshot({ path: testInfo.outputPath('desktop-light.png') });

    await page.setViewportSize({ width: 1200, height: 480 });
    await page.locator('cb-sidebar').getByRole('link', { name: /All Contacts/ }).click();
    await expect(page.locator('cb-page-contacts cb-contact-card')).toHaveCount(15);
    await expect(page.locator('cb-header')).toHaveAttribute('mode', 'desktop');
    const content = page.locator('cb-app .content');
    await content.evaluate(element => { element.scrollTop = element.scrollHeight; });
    expect(await content.evaluate(element => element.scrollTop)).toBeGreaterThan(0);
    expect((await page.locator('cb-header').boundingBox())?.y).toBe(0);
    await expectNativeControlSafeAreas(page);
    await expectViewportContained(page);
    await page.locator('cb-header').getByRole('link', { name: 'Add Contact', exact: true }).click();
    await expect(page.locator('cb-contact-form .form-title')).toBeInViewport();
    await expect(content).toHaveJSProperty('scrollTop', 0);
  });

  test('header actions support the keyboard without sending native window messages', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('cb-header')).toHaveJSProperty('$ready', true);
    await expectNativeControlSafeAreas(page);
    await expect(page.locator('cb-header [webui-no-drag] .search-input')).toHaveCount(1);
    const search = page.locator('cb-header .search-input');
    await search.focus();
    await expect(search).toBeFocused();
    await search.fill('Sarah');
    await expect(search).toHaveValue('Sarah');
    await expect(page.locator('cb-app')).toHaveJSProperty('searchQuery', 'Sarah');
    await page.locator('cb-header .add-btn').focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('cb-contact-form .form-title')).toBeInViewport();
    expect(await page.evaluate(() => (window as WindowBridge).windowActions)).toEqual([]);
    await page.getByRole('link', { name: 'Skip to content' }).focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('cb-app .content')).toBeFocused();
  });

  test('dark and narrow layouts reserve native caption space', async ({ page }, testInfo) => {
    await page.emulateMedia({ colorScheme: 'dark' });
    await page.goto('/');
    await expect(page.locator('cb-header')).toHaveJSProperty('$ready', true);
    await expectViewportContained(page);
    await expectNativeControlSafeAreas(page);
    await page.screenshot({ path: testInfo.outputPath('desktop-dark.png') });
    await page.setViewportSize({ width: 480, height: 640 });
    await expectNativeControlSafeAreas(page);
    await expect(page.locator('cb-header .search-input')).toBeVisible();
    await expect(page.locator('cb-header .add-btn')).toBeVisible();
    await expectViewportContained(page);
    await page.screenshot({ path: testInfo.outputPath('desktop-narrow.png') });
  });

  for (const [platform, insets] of [
    ['Windows', windowsInsets],
    ['macOS', { start: 78, end: 0, height: 48 }],
    ['Linux', { start: 0, end: 0, height: 48 }],
    ['custom', { start: 90, end: 160, height: 72 }],
  ] as const) {
    test(`${platform} safe areas apply to wide and responsive desktop headers`, async ({ page, request, baseURL }) => {
      await renderDesktopMode(page, request, baseURL, insets);
      await page.goto('/');
      await expect(page.locator('cb-header')).toHaveJSProperty('$ready', true);
      await expectNativeControlSafeAreas(page, insets);
      for (const width of [800, 768, 480, 390]) {
        await page.setViewportSize({ width, height: 640 });
        await expectNativeControlSafeAreas(page, insets);
        await expectViewportContained(page);
      }
    });
  }
});
