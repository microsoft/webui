// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';
import type { DeferredDialog } from '../src/deferred-dialog/deferred-dialog.js';
import type { DeferredPopover } from '../src/deferred-popover/deferred-popover.js';

async function ready(page: Page, path = '/'): Promise<void> {
  await page.goto(path);
  await hydrated(page);
}

async function hydrated(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const workspace = document.querySelector('deferred-workspace');
    return [
      workspace,
      workspace?.shadowRoot?.querySelector('deferred-popover'),
      workspace?.shadowRoot?.querySelector('deferred-dialog'),
    ].every(element => element && Reflect.get(element, '$ready') === true);
  });
}

function isState(url: string): boolean {
  return new URL(url).pathname === '/_webui/deferred/state';
}

test('initial SSR omits all overlay contents and their deferred metadata', async ({ page }) => {
  const response = await page.goto('/');
  const html = await response!.text();
  expect(html).not.toContain('Review queue');
  expect(html).not.toContain('Reading layout');
  expect(html).not.toContain('A quick look at this sample workspace.');
  expect(html).not.toContain('overlay-header');
  expect(html).not.toContain('Close activity');
  expect(html).not.toContain('Close settings');
  expect(html).not.toContain('Make this workspace easier to read.');
  await hydrated(page);
  await expect(page.locator('deferred-popover')).toHaveCount(1);
  await expect(page.locator('deferred-dialog')).toHaveCount(1);
  await expect(page.locator('.document-row')).toHaveCount(3);
  await expect(page.locator('.activity-body')).toHaveCount(0);
  await expect(page.locator('.settings-body')).toHaveCount(0);
  for (const selector of ['#activity-popover', '.settings-dialog']) {
    expect(await page.locator(selector).evaluate(element => element.children.length)).toBe(0);
  }
});

test('hover preloads the popover without showing it or fetching state', async ({ page }) => {
  await ready(page);
  let stateRequests = 0;
  page.on('request', request => { if (isState(request.url())) stateRequests++; });
  const metadata = page.waitForResponse(response => {
    const url = new URL(response.url());
    return url.pathname === '/_webui/deferred' && url.searchParams.get('id') === 'activity';
  });
  await page.getByRole('button', { name: 'Recent activity', exact: true }).hover();
  const response = await metadata;
  expect(response.ok()).toBe(true);
  const payload = await response.json();
  expect(payload).toMatchObject({
    owner: 'deferred-popover', fragment: 'deferred-popover::defer:activity',
  });
  expect(payload.templates['deferred-popover::defer:activity'].h).toContain('overlay-header');
  expect(payload.templates['deferred-popover::defer:activity'].h).toContain('Close activity');
  await page.locator('deferred-popover').evaluate(element => (element as DeferredPopover).preload());
  await expect(page.locator('#activity-popover')).not.toBeVisible();
  await expect(page.locator('.activity-body')).toHaveCount(0);
  await expect(page.locator('#activity-popover .overlay-header')).toHaveCount(0);
  expect(stateRequests).toBe(0);
});

test('a warm native popover requests fresh state without repeating its template', async ({ page }) => {
  await ready(page);
  await page.locator('deferred-popover').evaluate(element => (element as DeferredPopover).preload());
  const state = page.waitForResponse(response => isState(response.url()));
  await page.getByRole('button', { name: 'Recent activity', exact: true }).click();
  const records = (await (await state).text()).trim().split('\n').map(line => JSON.parse(line));
  expect(records).toHaveLength(2);
  expect(records.every(record => record.owner === 'deferred-popover')).toBe(true);
  expect(records.every(record => record.templates === undefined)).toBe(true);
  await expect(page.locator('.activity-list li')).toHaveCount(3);
  const stamp = await page.locator('.activity-body .response-stamp').textContent();
  await page.keyboard.press('Escape');
  await expect(page.locator('.activity-body')).toHaveCount(0);
  await page.getByRole('button', { name: 'Recent activity', exact: true }).click();
  await expect(page.locator('.activity-body .response-stamp')).not.toHaveText(stamp!);
});

test('native popover light-dismiss removes the deferred body', async ({ page }) => {
  await ready(page);
  await page.getByRole('button', { name: 'Recent activity', exact: true }).click();
  await expect(page.locator('.activity-body')).toBeVisible();
  await page.getByRole('heading', { name: 'Workspace', exact: true }).click();
  await expect(page.locator('#activity-popover')).not.toBeVisible();
  await expect(page.locator('.activity-body')).toHaveCount(0);
});

test('the dialog emits preferences to the workspace and receives updated attributes', async ({ page }) => {
  await ready(page);
  const opener = page.getByRole('button', { name: 'View settings', exact: true });
  const state = page.waitForResponse(response => isState(response.url()));
  await opener.click();
  const dialog = page.getByRole('dialog', { name: 'View settings' });
  const records = (await (await state).text()).trim().split('\n').map(line => JSON.parse(line));
  expect(records.every(record => record.owner === 'deferred-dialog')).toBe(true);
  await expect(dialog).toBeVisible();
  expect(await dialog.evaluate(element => element instanceof HTMLDialogElement && element.matches(':modal'))).toBe(true);
  await expect(dialog.getByLabel('Item spacing')).toBeVisible();
  await dialog.getByLabel('Item spacing').selectOption('compact');
  await dialog.getByLabel('Highlight pinned items').uncheck();
  await dialog.getByRole('button', { name: 'Apply to this page' }).click();
  await expect(dialog).not.toBeVisible();
  await expect(page.locator('.settings-body')).toHaveCount(0);
  await expect(page.locator('.workspace')).toHaveAttribute('data-compact', '');
  await expect(page.locator('.workspace')).not.toHaveAttribute('data-highlight', '');
  await expect(page.getByRole('status').filter({ hasText: 'View preferences applied' })).toBeVisible();
  await expect(opener).toBeFocused();
  await expect(page.locator('deferred-dialog')).toHaveAttribute('compact-mode', '');
  await expect(page.locator('deferred-dialog')).not.toHaveAttribute('highlight-pinned', '');
  await opener.click();
  await expect(dialog.getByLabel('Item spacing')).toHaveValue('compact');
  await expect(dialog.getByLabel('Highlight pinned items')).not.toBeChecked();
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
  await page.locator('deferred-dialog').evaluate(async element => {
    const component = element as DeferredDialog;
    component.dismiss();
    await component.open();
  });
  await expect(page.locator('deferred-dialog .settings-body')).toBeVisible();
});

for (const overlay of ['activity', 'settings'] as const) {
  for (const cancel of ['Escape', 'trigger'] as const) {
    test(`${overlay} stays closed and empty while loading, cancellable by ${cancel}`, async ({ page }) => {
      await ready(page);
      await page.getByLabel('Simulate slow data').check();
      let requests = 0;
      page.on('request', request => { if (isState(request.url())) requests++; });
      const response = page.waitForResponse(value => isState(value.url()));
      await page.getByRole('button', {
        name: overlay === 'activity' ? 'Recent activity' : 'View settings', exact: true,
      }).click();
      await response;
      const component = page.locator(overlay === 'activity' ? 'deferred-popover' : 'deferred-dialog');
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
    const response = page.waitForResponse(value => isState(value.url()));
    await page.getByRole('button', {
      name: overlay === 'activity' ? 'Recent activity' : 'View settings', exact: true,
    }).click();
    await response;
    const component = overlay === 'activity' ? 'deferred-popover' : 'deferred-dialog';
    const cancelled = page.waitForEvent('requestfailed', request => isState(request.url()));
    await page.locator(component).evaluate(element => element.remove());
    await cancelled;
    await expect(page.locator(overlay === 'activity' ? 'deferred-dialog' : 'deferred-popover')).toHaveCount(1);
    await expect(page.getByRole('heading', { name: 'Workspace', exact: true })).toBeVisible();
  });
}

for (const [tag, label, selector, error] of [
  ['deferred-popover', 'Recent activity', '#activity-popover', 'Activity could not be loaded'],
  ['deferred-dialog', 'View settings', '.settings-dialog', 'Settings could not be loaded'],
]) {
  test(`${tag} reports state failures outside its empty overlay and retries`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'light' });
    await ready(page);
    let failed = false;
    await page.route('**/_webui/deferred/state?**', async route => {
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

for (const overlay of ['popover', 'dialog'] as const) {
  test(`an initially active ${overlay} uses SSR without another state request`, async ({ page }) => {
    let requests = 0;
    page.on('request', request => { if (isState(request.url())) requests++; });
    const response = await page.goto(`/?open=${overlay}`);
    const html = await response!.text();
    expect(html).toContain(overlay === 'popover' ? 'class="activity-body"' : 'class="settings-body"');
    expect(html).toContain('class="overlay-header"');
    await hydrated(page);
    await expect(page.locator(overlay === 'popover' ? '.activity-body' : '.settings-body')).toBeVisible();
    expect(requests).toBe(0);
    await page.keyboard.press('Escape');
    await expect(page.locator(overlay === 'popover' ? '.activity-body' : '.settings-body')).toHaveCount(0);
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
  const component = page.locator('deferred-popover');
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

test('a component preload failure reaches the workspace announcement', async ({ page }) => {
  await ready(page);
  await page.route('**/_webui/deferred?**', route => route.fulfill({ status: 503, body: 'Unavailable' }));
  await page.getByRole('button', { name: 'Recent activity', exact: true }).hover();
  await expect(page.getByRole('status').filter({ hasText: 'Preload was unavailable' })).toBeVisible();
  await page.unroute('**/_webui/deferred?**');
  await page.getByRole('button', { name: 'Recent activity', exact: true }).click();
  await expect(page.locator('deferred-popover .activity-body')).toBeVisible();
});

test('resource and state requests respect the demo catalog base path', async ({ page }) => {
  const prefix = '/deferred-ui/';
  const outsideBase: string[] = [];
  await page.route('**/_webui/**', async route => {
    outsideBase.push(route.request().url());
    await route.fulfill({ status: 404, body: 'Use the app base path.' });
  });
  await page.route('**/deferred-ui/**', async route => {
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
