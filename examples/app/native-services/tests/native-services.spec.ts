// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

const capability = '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef';
const demoUrl = `/?native-capability=${capability}`;

test('reports picker cancellation without treating it as an error', async ({ page }) => {
  await page.route('**/api/picker', route => {
    expect(route.request().headers()['x-webui-native-capability']).toBe(capability);
    return route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ status: 'Picker cancelled', detail: 'No directory was selected.' }),
    });
  });
  await page.goto(demoUrl);
  await page.getByRole('button', { name: 'Choose directory' }).click();
  await expect(page.getByRole('heading', { name: 'Picker cancelled' })).toBeVisible();
  await expect(page.getByText('No directory was selected.')).toBeVisible();
});

test('previews capture and enables clipboard only after success', async ({ page }) => {
  await page.route('**/api/capture', route => route.fulfill({
    contentType: 'image/png',
    body: Buffer.from(
      'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADElEQVR42mP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC',
      'base64',
    ),
  }));
  await page.goto(demoUrl);
  const copy = page.getByRole('button', { name: 'Copy latest capture' });
  await expect(copy).toBeDisabled();
  await page.getByRole('button', { name: 'Capture this view' }).click();
  await expect(page.getByRole('heading', { name: 'Capture ready' })).toBeVisible();
  await expect(page.getByAltText('Latest visible native webview capture')).toBeVisible();
  await expect(copy).toBeEnabled();
});

test('clears a prior capture when recapture fails', async ({ page }) => {
  let attempts = 0;
  await page.route('**/api/capture', route => {
    attempts += 1;
    if (attempts === 1) {
      return route.fulfill({
        contentType: 'image/png',
        body: Buffer.from(
          'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADElEQVR42mP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC',
          'base64',
        ),
      });
    }
    return route.fulfill({
      status: 503,
      contentType: 'application/json',
      body: JSON.stringify({ status: 'Unavailable', detail: 'The second capture failed.' }),
    });
  });
  await page.goto(demoUrl);
  const capture = page.getByRole('button', { name: 'Capture this view' });
  const copy = page.getByRole('button', { name: 'Copy latest capture' });
  await capture.click();
  await expect(copy).toBeEnabled();
  await capture.click();
  await expect(page.getByRole('heading', { name: 'Action failed' })).toBeVisible();
  await expect(page.getByAltText('Latest visible native webview capture')).toBeHidden();
  await expect(copy).toBeDisabled();
});

test('surfaces a typed host failure and restores controls', async ({ page }) => {
  await page.route('**/api/dialog/error', route => route.fulfill({
    status: 503,
    contentType: 'application/json',
    body: JSON.stringify({ status: 'Unavailable', detail: 'Native dialogs are unsupported on this platform.' }),
  }));
  await page.goto(demoUrl);
  await page.getByRole('button', { name: 'Show error' }).click();
  await expect(page.getByRole('heading', { name: 'Action failed' })).toBeVisible();
  await expect(page.getByText('Native dialogs are unsupported on this platform.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Show error' })).toBeEnabled();
});
