// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import type { LazyDialog } from '../src/lazy-dialog/lazy-dialog.js';
import {
  exampleRoot,
  startAssetOrigin,
  startExampleServer,
  type AssetOrigin,
  type TestServer,
} from './servers.js';

/** The sample API that `start:api` runs and the example server proxies. */
const API_ORIGIN = 'http://127.0.0.1:3021';
const PREVIEW_CAPTION = /^Preview: \d+ items fit in this space\.$/;

async function ready(page: Page, path = '/'): Promise<void> {
  await page.goto(path);
  await hydrated(page);
}

/** Load the page and wait for the idle prefetch of the activity pack. */
async function settled(page: Page): Promise<void> {
  const prefetched = page.waitForResponse(response => isPackOf('lazy-popover', response.url()));
  await ready(page);
  expect((await prefetched).ok()).toBe(true);
}

async function hydrated(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const workspace = document.querySelector('lazy-workspace');
    return [
      workspace,
      workspace?.shadowRoot?.querySelector('lazy-popover'),
      workspace?.shadowRoot?.querySelector('lazy-dialog'),
    ].every(element => element && Reflect.get(element, '$ready') === true);
  });
}

function isState(url: string): boolean {
  return /^\/api\/(activity|settings)$/.test(new URL(url).pathname);
}

function isPack(url: string): boolean {
  return new URL(url).pathname.startsWith('/_webui/packs/');
}

/** The pack of one owner's lazy region: `_webui/packs/<owner>.<hash>.json`. */
function isPackOf(owner: 'lazy-popover' | 'lazy-dialog', url: string): boolean {
  return new RegExp(`^/_webui/packs/${owner}\\.[0-9a-f]{16}\\.json$`).test(new URL(url).pathname);
}

/** The chunk the bundler splits from `src/layout-preview/layout-preview.ts`. */
function isPreviewScript(url: string): boolean {
  return /\/layout-preview-[A-Z0-9]+\.js$/.test(new URL(url).pathname);
}

/** Uncaught errors, including errors passed to `reportError()`. */
function collectPageErrors(page: Page): Error[] {
  const errors: Error[] = [];
  page.on('pageerror', error => errors.push(error));
  return errors;
}

/** Record each `lazyerror` that reaches the window, and unhandled rejections. */
async function recordLazyErrors(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const events: unknown[] = [];
    const rejections: string[] = [];
    Reflect.set(window, '__lazyErrors', events);
    Reflect.set(window, '__rejections', rejections);
    addEventListener('lazyerror', event => {
      const { detail } = event as CustomEvent<{ error: unknown }>;
      events.push({
        owner: (event.composedPath()[0] as Element).localName,
        handled: event.defaultPrevented,
        error: String(detail.error),
      });
    });
    addEventListener('unhandledrejection', event => { rejections.push(String(event.reason)); });
  });
}

function recorded(page: Page, name: '__lazyErrors' | '__rejections'): Promise<unknown[]> {
  return page.evaluate(key => Reflect.get(window, key) as unknown[], name);
}

function countIn(caption: string | null): number {
  return Number(/\d+/.exec(caption ?? '')?.[0]);
}

test('initial SSR omits overlay contents and only references their packs', async ({ page }) => {
  const response = await page.goto('/');
  const html = await response!.text();
  expect(html).toMatch(/"lp":\{"lazy-popover::lazy:@0":"_webui\/packs\/lazy-popover\.[0-9a-f]{16}\.json"\}/);
  expect(html).toMatch(/"lp":\{"lazy-dialog::lazy:@0":"_webui\/packs\/lazy-dialog\.[0-9a-f]{16}\.json"\}/);
  expect(html).not.toContain('Review queue');
  expect(html).not.toContain('Reading layout');
  expect(html).not.toContain('A quick look at this sample workspace.');
  expect(html).not.toContain('overlay-header');
  expect(html).not.toContain('Close activity');
  expect(html).not.toContain('Close settings');
  expect(html).not.toContain('Make this workspace easier to read.');
  // Only the settings body uses the preview: no template, stylesheet, or script for it.
  expect(html).not.toContain('layout-preview');
  await hydrated(page);
  await expect(page.locator('lazy-popover')).toHaveCount(1);
  await expect(page.locator('lazy-dialog')).toHaveCount(1);
  await expect(page.locator('.document-row')).toHaveCount(3);
  await expect(page.locator('.activity-body')).toHaveCount(0);
  await expect(page.locator('.settings-body')).toHaveCount(0);
  for (const selector of ['#activity-popover', '.settings-dialog']) {
    expect(await page.locator(selector).evaluate(element => element.children.length)).toBe(0);
  }
});

test('the activity pack is prefetched after the page loads, without rendering or state', async ({ page }) => {
  let stateRequests = 0;
  page.on('request', request => { if (isState(request.url())) stateRequests++; });
  const pack = page.waitForResponse(response => isPackOf('lazy-popover', response.url()));
  await ready(page);
  const response = await pack;
  expect(response.ok()).toBe(true);
  const payload = await response.json();
  expect(payload).toMatchObject({ type: 'webui-pack', version: 1, fragment: 'lazy-popover::lazy:@0' });
  expect(payload.state).toBeUndefined();
  expect(payload.templates['lazy-popover::lazy:@0'].h).toContain('overlay-header');
  expect(payload.templates['lazy-popover::lazy:@0'].h).toContain('Close activity');
  // `lazy="idle"` waits for the window load event.
  const timing = await page.evaluate(url => {
    const [navigation] = performance.getEntriesByType('navigation') as PerformanceNavigationTiming[];
    const [request] = performance.getEntriesByName(url, 'resource');
    return { loaded: navigation.loadEventEnd, requested: request?.startTime ?? -1 };
  }, response.url());
  expect(timing.requested).toBeGreaterThanOrEqual(timing.loaded);
  await expect(page.locator('#activity-popover')).not.toBeVisible();
  await expect(page.locator('.activity-body')).toHaveCount(0);
  await expect(page.locator('#activity-popover .overlay-header')).toHaveCount(0);
  expect(stateRequests).toBe(0);
});

test('a warm native popover requests fresh state without refetching its pack', async ({ page }) => {
  await settled(page);
  const packs: string[] = [];
  page.on('request', request => { if (isPack(request.url())) packs.push(request.url()); });
  const state = page.waitForResponse(response => isState(response.url()));
  await page.getByRole('button', { name: 'Recent activity', exact: true }).click();
  const body = await (await state).json();
  expect(body.state.activityItems).toHaveLength(3);
  expect(body.templates).toBeUndefined();
  await expect(page.locator('.activity-list li')).toHaveCount(3);
  expect(packs).toEqual([]);
  const stamp = await page.locator('.activity-body .response-stamp').textContent();
  await page.keyboard.press('Escape');
  await expect(page.locator('.activity-body')).toHaveCount(0);
  await page.getByRole('button', { name: 'Recent activity', exact: true }).click();
  await expect(page.locator('.activity-body .response-stamp')).not.toHaveText(stamp!);
});

test('native popover light-dismiss removes the lazy body', async ({ page }) => {
  await ready(page);
  await page.getByRole('button', { name: 'Recent activity', exact: true }).click();
  await expect(page.locator('.activity-body')).toBeVisible();
  await page.getByRole('heading', { name: 'Workspace', exact: true }).click();
  await expect(page.locator('#activity-popover')).not.toBeVisible();
  await expect(page.locator('.activity-body')).toHaveCount(0);
});

for (const intent of ['pointer', 'keyboard'] as const) {
  test(`${intent} intent on View settings prefetches only the settings pack`, async ({ page }) => {
    await settled(page);
    const packs: string[] = [];
    page.on('request', request => { if (isPack(request.url())) packs.push(new URL(request.url()).pathname); });
    // The settings region is `lazy="demand"`, so the rest of the page prefetches nothing.
    await page.getByRole('heading', { name: 'Saved for later' }).hover();
    await page.getByRole('button', { name: 'Recent activity', exact: true }).hover();
    await page.getByLabel('Simulate slow data').focus();
    const trigger = page.getByRole('button', { name: 'View settings', exact: true });
    const prefetched = page.waitForResponse(response => isPackOf('lazy-dialog', response.url()));
    if (intent === 'pointer') await trigger.hover();
    else await trigger.focus();
    expect((await prefetched).ok()).toBe(true);
    await expect(page.locator('.settings-body')).toHaveCount(0);
    if (intent === 'pointer') await trigger.click();
    else await page.keyboard.press('Enter');
    await expect(page.getByRole('dialog', { name: 'View settings' })).toBeVisible();
    await expect(page.locator('.settings-body')).toBeVisible();
    expect(packs).toEqual([expect.stringMatching(/^\/_webui\/packs\/lazy-dialog\.[0-9a-f]{16}\.json$/)]);
  });
}

test('the preview script loads on demand and the preview is upgraded when its body appears', async ({ page }) => {
  const scripts: string[] = [];
  const warnings: string[] = [];
  page.on('request', request => { if (isPreviewScript(request.url())) scripts.push(request.url()); });
  page.on('console', message => { if (message.type() === 'warning') warnings.push(message.text()); });
  // Deliver the script after the pack and the state, so the body has to wait for it.
  await page.route(url => isPreviewScript(url.href), async route => {
    await new Promise(resolve => setTimeout(resolve, 400));
    await route.continue();
  });
  await settled(page);
  expect(scripts).toEqual([]);
  expect(await page.evaluate(() => customElements.get('layout-preview') === undefined)).toBe(true);

  // Record the preview when it is inserted, and its caption once that frame has painted.
  await page.locator('lazy-dialog').evaluate(host => {
    const shadow = host.shadowRoot!;
    Reflect.set(window, '__previewAppearance', new Promise(resolve => {
      const observer = new MutationObserver(() => {
        const preview = shadow.querySelector('layout-preview');
        if (!preview) return;
        observer.disconnect();
        const definition = customElements.get('layout-preview');
        const inserted = {
          upgraded: definition !== undefined && preview instanceof definition,
          mounted: preview.shadowRoot !== null,
        };
        requestAnimationFrame(() => setTimeout(() => resolve({
          ...inserted,
          dialogOpen: shadow.querySelector('dialog')?.open === true,
          caption: preview.shadowRoot?.querySelector('figcaption')?.textContent ?? '',
        })));
      });
      observer.observe(shadow, { childList: true, subtree: true });
    }));
  });
  await page.getByRole('button', { name: 'View settings', exact: true }).click();
  expect(await page.evaluate(() => Reflect.get(window, '__previewAppearance'))).toEqual({
    upgraded: true,
    mounted: true,
    dialogOpen: true,
    caption: expect.stringMatching(PREVIEW_CAPTION),
  });
  expect(scripts).toHaveLength(1);

  // The script measures how many rows fit, so the caption follows the spacing.
  const caption = page.locator('layout-preview figcaption');
  const comfortable = countIn(await caption.textContent());
  await page.getByLabel('Item spacing').selectOption('compact');
  await expect(page.locator('layout-preview')).toHaveAttribute('density', 'compact');
  await expect(caption).not.toHaveText(`Preview: ${comfortable} items fit in this space.`);
  expect(countIn(await caption.textContent())).toBeGreaterThan(comfortable);

  // Reopening reuses the definition, and nothing reported a startup definition.
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'View settings', exact: true }).click();
  await expect(caption).toHaveText(PREVIEW_CAPTION);
  expect(scripts).toHaveLength(1);
  expect(warnings.filter(text => text.includes('only used inside lazy regions'))).toEqual([]);
});

test('the dialog emits preferences to the workspace and receives updated attributes', async ({ page }) => {
  await ready(page);
  const opener = page.getByRole('button', { name: 'View settings', exact: true });
  const state = page.waitForResponse(response => isState(response.url()));
  await opener.click();
  const dialog = page.getByRole('dialog', { name: 'View settings' });
  expect((await (await state).json()).state.layoutOptions).toHaveLength(2);
  await expect(dialog).toBeVisible();
  expect(await dialog.evaluate(element => element instanceof HTMLDialogElement && element.matches(':modal'))).toBe(true);
  await expect(dialog.getByLabel('Item spacing')).toBeVisible();
  await dialog.getByLabel('Item spacing').selectOption('compact');
  await dialog.getByLabel('Highlight pinned items').uncheck();
  await expect(page.locator('layout-preview')).not.toHaveAttribute('highlight', '');
  await dialog.getByRole('button', { name: 'Apply to this page' }).click();
  await expect(dialog).not.toBeVisible();
  await expect(page.locator('.settings-body')).toHaveCount(0);
  await expect(page.locator('.workspace')).toHaveAttribute('data-compact', '');
  await expect(page.locator('.workspace')).not.toHaveAttribute('data-highlight', '');
  await expect(page.getByRole('status').filter({ hasText: 'View preferences applied' })).toBeVisible();
  await expect(opener).toBeFocused();
  await expect(page.locator('lazy-dialog')).toHaveAttribute('compact-mode', '');
  await expect(page.locator('lazy-dialog')).not.toHaveAttribute('highlight-pinned', '');
  await opener.click();
  await expect(dialog.getByLabel('Item spacing')).toHaveValue('compact');
  await expect(dialog.getByLabel('Highlight pinned items')).not.toBeChecked();
  await expect(page.locator('layout-preview')).toHaveAttribute('density', 'compact');
});

test('cancelling dialog edits preserves the current view and resets the draft', async ({ page }) => {
  await ready(page);
  const opener = page.getByRole('button', { name: 'View settings', exact: true });
  await opener.click();
  await page.getByLabel('Item spacing').selectOption('compact');
  await page.getByLabel('Highlight pinned items').uncheck();
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.locator('.workspace')).not.toHaveAttribute('data-compact', '');
  await expect(page.locator('.workspace')).toHaveAttribute('data-highlight', '');
  await opener.click();
  await expect(page.getByLabel('Item spacing')).toHaveValue('comfortable');
  await expect(page.getByLabel('Highlight pinned items')).toBeChecked();
});

test('an old native close event does not cancel a newly reopened dialog', async ({ page }) => {
  await ready(page);
  await page.getByRole('button', { name: 'View settings', exact: true }).click();
  await expect(page.locator('.settings-body')).toBeVisible();
  await page.locator('lazy-dialog').evaluate(async element => {
    const component = element as LazyDialog;
    component.dismiss();
    await component.open();
  });
  await expect(page.locator('lazy-dialog .settings-body')).toBeVisible();
});

for (const overlay of ['activity', 'settings'] as const) {
  for (const cancel of ['Escape', 'trigger'] as const) {
    test(`${overlay} stays closed and empty while loading, cancellable by ${cancel}`, async ({ page }) => {
      await ready(page);
      await page.getByLabel('Simulate slow data').check();
      let requests = 0;
      page.on('request', request => { if (isState(request.url())) requests++; });
      const pending = page.waitForRequest(value => isState(value.url()));
      await page.getByRole('button', {
        name: overlay === 'activity' ? 'Recent activity' : 'View settings', exact: true,
      }).click();
      await pending;
      const component = page.locator(overlay === 'activity' ? 'lazy-popover' : 'lazy-dialog');
      const shell = component.locator(overlay === 'activity' ? '#activity-popover' : '.settings-dialog');
      await expect(component.locator('.loading-state')).toBeVisible();
      await expect(shell).not.toBeVisible();
      expect(await shell.evaluate(element => element.children.length)).toBe(0);
      const trigger = component.getByRole('button', { name: `Cancel ${overlay} load` });
      await expect(trigger).toHaveAttribute('aria-busy', 'true');
      if (cancel === 'Escape') await page.keyboard.press('Escape');
      else await trigger.click();
      await expect(component.locator('.loading-state')).toHaveCount(0);
      await page.waitForTimeout(650);
      expect(await shell.evaluate(element => element.children.length)).toBe(0);
      await expect(shell).not.toBeVisible();
      expect(requests).toBe(1);
    });
  }

  test(`${overlay} disconnect aborts its pending state request`, async ({ page }) => {
    await ready(page);
    await page.getByLabel('Simulate slow data').check();
    const pending = page.waitForRequest(value => isState(value.url()));
    await page.getByRole('button', {
      name: overlay === 'activity' ? 'Recent activity' : 'View settings', exact: true,
    }).click();
    await pending;
    const component = overlay === 'activity' ? 'lazy-popover' : 'lazy-dialog';
    const cancelled = page.waitForEvent('requestfailed', request => isState(request.url()));
    await page.locator(component).evaluate(element => element.remove());
    await cancelled;
    await expect(page.locator(overlay === 'activity' ? 'lazy-dialog' : 'lazy-popover')).toHaveCount(1);
    await expect(page.getByRole('heading', { name: 'Workspace', exact: true })).toBeVisible();
  });
}

for (const [tag, label, selector, error] of [
  ['lazy-popover', 'Recent activity', '#activity-popover', 'Activity could not be loaded'],
  ['lazy-dialog', 'View settings', '.settings-dialog', 'Settings could not be loaded'],
]) {
  test(`${tag} reports state failures outside its empty overlay and retries`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'light' });
    await ready(page);
    let failed = false;
    await page.route('**/api/**', async route => {
      if (!failed) {
        failed = true;
        await route.fulfill({
          status: 503, contentType: 'application/json',
          body: '{"error":{"code":"sample-unavailable","message":"Try again"}}',
        });
      } else {
        await route.continue();
      }
    });
    const component = page.locator(tag);
    const overlay = component.locator(selector);
    await component.getByRole('button', { name: label, exact: true }).click();
    await expect(component.getByRole('alert')).toContainText(error);
    const contrast = await component.locator('.error-state p').evaluate(element => {
      const luminance = (color: string): number => {
        const channels = color.match(/\d+/g);
        if (!channels) throw new Error(`Expected an RGB color, received ${color}`);
        const linear = channels.map(channel => {
          const value = Number(channel) / 255;
          return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
        });
        return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
      };
      const foreground = luminance(getComputedStyle(element).color);
      const background = luminance(getComputedStyle(document.body).backgroundColor);
      return (Math.max(foreground, background) + 0.05) / (Math.min(foreground, background) + 0.05);
    });
    expect(contrast).toBeGreaterThanOrEqual(4.5);
    await expect(overlay).not.toBeVisible();
    expect(await overlay.evaluate(element => element.children.length)).toBe(0);
    await component.getByRole('button', { name: 'Try again' }).click();
    await expect(overlay).toBeVisible();
    await expect(overlay.locator('.overlay-header')).toBeVisible();
    expect(new URL(page.url()).pathname).toBe('/');
  });
}

test('a blocked settings pack raises a handled lazyerror, and a retry opens the dialog', async ({ page }) => {
  const pageErrors = collectPageErrors(page);
  await recordLazyErrors(page);
  await page.route('**/_webui/packs/lazy-dialog.*', route => route.abort());
  await settled(page);
  const component = page.locator('lazy-dialog');
  const trigger = component.getByRole('button', { name: 'View settings', exact: true });
  await trigger.click();
  await expect(component.getByRole('alert')).toContainText('Settings could not be loaded');
  // The owner cancelled the event, kept the dialog closed, and reset its condition.
  expect(await recorded(page, '__lazyErrors')).toEqual([
    { owner: 'lazy-dialog', handled: true, error: expect.stringContaining('Failed to fetch') },
  ]);
  expect(await component.evaluate(element => (element as LazyDialog).settingsReady)).toBe(false);
  await expect(trigger).toHaveAttribute('aria-busy', 'false');
  await expect(component.locator('.settings-dialog')).not.toBeVisible();
  expect(await component.locator('.settings-dialog').evaluate(element => element.children.length)).toBe(0);

  await page.unroute('**/_webui/packs/lazy-dialog.*');
  await component.getByRole('button', { name: 'Try again' }).click();
  await expect(page.getByRole('dialog', { name: 'View settings' })).toBeVisible();
  await expect(component.locator('.settings-body')).toBeVisible();
  await expect(component.getByRole('alert')).toHaveCount(0);
  expect(await recorded(page, '__lazyErrors')).toHaveLength(1);
  expect(await recorded(page, '__rejections')).toEqual([]);
  expect(pageErrors).toEqual([]);
});

test('a settings pack that fails after the user cancels stays quiet', async ({ page }) => {
  const pageErrors = collectPageErrors(page);
  await recordLazyErrors(page);
  let failPack = (): void => {};
  const failing = new Promise<void>(resolve => { failPack = resolve; });
  await page.route('**/_webui/packs/lazy-dialog.*', async route => {
    await failing;
    await route.abort();
  });
  await settled(page);
  const component = page.locator('lazy-dialog');
  await component.getByRole('button', { name: 'View settings', exact: true }).click();
  // The state has arrived and the body is waiting for its pack.
  await expect.poll(() => component.evaluate(element => (element as LazyDialog).settingsReady)).toBe(true);
  await expect(component.getByRole('button', { name: 'Cancel settings load' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(component.locator('.loading-state')).toHaveCount(0);
  failPack();
  await expect.poll(() => recorded(page, '__lazyErrors')).toEqual([
    { owner: 'lazy-dialog', handled: true, error: expect.stringContaining('Failed to fetch') },
  ]);
  await expect(component.getByRole('alert')).toHaveCount(0);
  await expect(component.getByRole('button', { name: 'View settings', exact: true })).toBeVisible();
  expect(await recorded(page, '__rejections')).toEqual([]);
  expect(pageErrors).toEqual([]);
});

for (const [overlay, owner, body] of [
  ['popover', 'lazy-popover', '.activity-body'],
  ['dialog', 'lazy-dialog', '.settings-body'],
] as const) {
  test(`an initially active ${overlay} uses SSR without requesting its state or pack`, async ({ page }) => {
    let requests = 0;
    page.on('request', request => { if (isState(request.url()) || isPackOf(owner, request.url())) requests++; });
    const response = await page.goto(`/?open=${overlay}`);
    const html = await response!.text();
    expect(html).toContain(`class="${body.slice(1)}"`);
    expect(html).toContain('class="overlay-header"');
    await hydrated(page);
    await expect(page.locator(body)).toBeVisible();
    if (overlay === 'dialog') {
      // The server rendered the preview; its script loads during hydration.
      expect(html).toContain('<layout-preview');
      await expect(page.locator('layout-preview figcaption')).toHaveText(PREVIEW_CAPTION);
    }
    expect(requests).toBe(0);
    await page.keyboard.press('Escape');
    await expect(page.locator(body)).toHaveCount(0);
    await expect(page.locator('.overlay-header')).toHaveCount(0);
  });
}

test('clearing cached templates preserves the shell and permits reopening', async ({ page }) => {
  await ready(page);
  await page.getByRole('button', { name: 'View settings', exact: true }).click();
  await expect(page.locator('.settings-body')).toBeVisible();
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Clear template cache' }).click();
  await expect(page.locator('.document-row')).toHaveCount(3);
  await page.getByRole('button', { name: 'View settings', exact: true }).click();
  await expect(page.getByLabel('Item spacing')).toBeVisible();
  await expect(page.locator('layout-preview figcaption')).toHaveText(PREVIEW_CAPTION);
});

test('native overlays stay within a narrow viewport', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await ready(page);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  for (const [button, selector] of [
    ['Recent activity', '#activity-popover'],
    ['View settings', '.settings-dialog'],
  ]) {
    await page.getByRole('button', { name: button, exact: true }).click();
    await expect(page.locator(selector)).toBeVisible();
    const box = await page.locator(selector).boundingBox();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(376);
    await page.keyboard.press('Escape');
  }
});

test('the popover uses its component-local native invoker and CSS anchor', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await ready(page);
  const component = page.locator('lazy-popover');
  const trigger = component.getByRole('button', { name: 'Recent activity', exact: true });
  const popover = component.locator('#activity-popover');
  await expect(trigger).toHaveAttribute('popovertarget', 'activity-popover');
  await expect(popover).toHaveAttribute('popover', 'auto');
  await trigger.click();
  await expect(component.locator('.activity-body')).toBeVisible();
  expect(await popover.evaluate(element => element.matches(':popover-open'))).toBe(true);
  await expect(popover).toHaveCSS('position-anchor', '--activity-trigger');
  const button = await trigger.boundingBox();
  const panel = await popover.boundingBox();
  expect(panel!.x + panel!.width).toBeCloseTo(button!.x + button!.width, 0);
  expect(panel!.y - button!.y - button!.height).toBeCloseTo(8, 0);
  await component.getByRole('button', { name: 'Close activity' }).click();
  await expect(component.locator('.activity-body')).toHaveCount(0);
  await expect(component.locator('.overlay-header')).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

test('a failed idle prefetch stays silent, fails the open visibly, and retries', async ({ page }) => {
  const pageErrors = collectPageErrors(page);
  await page.route('**/_webui/packs/lazy-popover.*', route => route.fulfill({ status: 503, body: 'Unavailable' }));
  const prefetch = page.waitForResponse(response => isPackOf('lazy-popover', response.url()));
  await ready(page);
  expect((await prefetch).status()).toBe(503);
  await expect(page.getByRole('alert')).toHaveCount(0);
  await page.getByRole('button', { name: 'Recent activity', exact: true }).click();
  await expect(page.locator('lazy-popover').getByRole('alert')).toContainText('Activity could not be loaded');
  await expect(page.locator('#activity-popover')).not.toBeVisible();
  await page.unroute('**/_webui/packs/lazy-popover.*');
  await page.locator('lazy-popover').getByRole('button', { name: 'Try again' }).click();
  await expect(page.locator('lazy-popover .activity-body')).toBeVisible();
  // `load()` reported the failure, and the popover cancelled its `lazyerror`.
  expect(pageErrors).toEqual([]);
});

test('resource and state requests respect the demo catalog base path', async ({ page }) => {
  const prefix = '/lazy-ui/';
  const outsideBase: string[] = [];
  await page.route('**/_webui/**', async route => {
    outsideBase.push(route.request().url());
    await route.fulfill({ status: 404, body: 'Use the app base path.' });
  });
  await page.route('**/lazy-ui/**', async route => {
    const url = new URL(route.request().url());
    url.pathname = '/' + url.pathname.slice(prefix.length);
    const response = await route.fetch({ url: url.href });
    if (route.request().isNavigationRequest()) {
      const html = await response.text();
      const base = /<base href="[^"]*">/;
      expect(html).toMatch(base);
      await route.fulfill({ response, body: html.replace(base, `<base href="${prefix}">`) });
    } else {
      await route.fulfill({ response });
    }
  });
  await ready(page, prefix);
  for (const [button, selector] of [
    ['Recent activity', '.activity-body'],
    ['View settings', '.settings-body'],
  ]) {
    await page.getByRole('button', { name: button, exact: true }).click();
    await expect(page.locator(selector)).toBeVisible();
    await page.keyboard.press('Escape');
  }
  expect(outsideBase).toEqual([]);
});

test('speculative pack requests use low priority, and none is high before the load event', async ({ page }) => {
  const cdp = await page.context().newCDPSession(page);
  const packs: { path: string; priority: string; at: number }[] = [];
  let loadedAt = Number.POSITIVE_INFINITY;
  cdp.on('Network.requestWillBeSent', event => {
    if (!isPack(event.request.url)) return;
    packs.push({ path: new URL(event.request.url).pathname, priority: event.request.initialPriority, at: event.timestamp });
  });
  cdp.on('Page.loadEventFired', event => { loadedAt = event.timestamp; });
  await cdp.send('Network.enable');
  await cdp.send('Page.enable');
  // An image that the test releases keeps the window load event pending, as a
  // slow resource would, so the page is used before it has loaded.
  let release = (): void => {};
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/hold-load.gif', async route => {
    await held;
    await route.fulfill({ status: 204 });
  });
  await page.addInitScript(() => {
    addEventListener('DOMContentLoaded', () => {
      const image = new Image();
      image.alt = '';
      image.src = '/hold-load.gif';
      document.body.append(image);
    }, { once: true });
  });
  try {
    await page.goto('/', { waitUntil: 'domcontentloaded' });
    await hydrated(page);
    expect(await page.evaluate(() => document.readyState)).toBe('interactive');
    const settings = page.waitForRequest(request => isPackOf('lazy-dialog', request.url()));
    await page.getByRole('button', { name: 'View settings', exact: true }).hover();
    await settings;
    const activity = page.waitForRequest(request => isPackOf('lazy-popover', request.url()));
    release();
    await activity;
  } finally {
    release();
  }
  await expect.poll(() => loadedAt).toBeLessThan(Number.POSITIVE_INFINITY);
  // `$prefetch()` before the load event, then the idle prefetch after it: both low.
  expect(packs).toEqual([
    { path: expect.stringMatching(/^\/_webui\/packs\/lazy-dialog\./), priority: 'Low', at: expect.any(Number) },
    { path: expect.stringMatching(/^\/_webui\/packs\/lazy-popover\./), priority: 'Low', at: expect.any(Number) },
  ]);
  expect(packs[0].at).toBeLessThan(loadedAt);
  expect(packs[1].at).toBeGreaterThan(loadedAt);
  expect(packs.filter(pack => pack.at < loadedAt && pack.priority !== 'Low')).toEqual([]);
});

test.describe('a per-request asset base on another origin', () => {
  let assets: AssetOrigin | undefined;
  let app: TestServer | undefined;

  test.beforeAll(async ({}, testInfo) => {
    assets = await startAssetOrigin(join(exampleRoot, 'dist'));
    // The host sets the reserved `$webui.assetBase` member in the render state.
    const state = JSON.parse(await readFile(join(exampleRoot, 'data', 'state.json'), 'utf8')) as object;
    const stateFile = testInfo.outputPath('asset-base-state.json');
    await writeFile(stateFile, JSON.stringify({ ...state, $webui: { assetBase: `${assets.origin}/` } }));
    app = await startExampleServer(stateFile);
  });

  test.afterAll(async () => {
    await app?.stop();
    await assets?.stop();
  });

  test('stylesheets and packs load from the asset base, and a region opens styled', async ({ page }) => {
    const cdn = assets!;
    const origin = app!.origin;
    const fromPage: string[] = [];
    page.on('request', request => {
      const url = new URL(request.url());
      if (url.origin === origin) fromPage.push(url.pathname);
    });
    // This server renders from a state file; the sample API still supplies overlay state.
    await page.route(`${origin}/api/**`, async route => {
      const url = new URL(route.request().url());
      await route.fulfill({ response: await route.fetch({ url: `${API_ORIGIN}${url.pathname}${url.search}` }) });
    });
    const response = await page.goto(`${origin}/`);
    expect(await response!.text()).toContain(`"assetBase":"${cdn.origin}/"`);
    await hydrated(page);
    const stylesheet = await page.locator('lazy-dialog').evaluate(host =>
      host.shadowRoot?.querySelector<HTMLLinkElement>('link[rel="stylesheet"]')?.href);
    expect(stylesheet).toBe(`${cdn.origin}/lazy-dialog.css`);

    await page.getByRole('button', { name: 'View settings', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'View settings' })).toBeVisible();
    // Styles from the startup stylesheet and from the stylesheet the pack names.
    await expect(page.locator('.settings-body')).toHaveCSS('padding-top', '24px');
    await expect(page.locator('layout-preview .viewport')).toHaveCSS('height', '132px');
    await expect(page.locator('layout-preview figcaption')).toHaveText(PREVIEW_CAPTION);
    await expect.poll(() => cdn.requests.filter(path => path.startsWith('/_webui/packs/')).sort()).toEqual([
      expect.stringMatching(/^\/_webui\/packs\/lazy-dialog\.[0-9a-f]{16}\.json$/),
      expect.stringMatching(/^\/_webui\/packs\/lazy-popover\.[0-9a-f]{16}\.json$/),
    ]);
    expect(cdn.requests).toEqual(expect.arrayContaining([
      '/lazy-workspace.css', '/lazy-popover.css', '/lazy-dialog.css', '/layout-preview.css',
    ]));
    expect(fromPage.filter(path => path.endsWith('.css') || path.startsWith('/_webui/'))).toEqual([]);
  });
});
