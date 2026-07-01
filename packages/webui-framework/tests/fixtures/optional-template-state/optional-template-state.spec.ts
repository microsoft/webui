// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('optional template state fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/optional-template-state/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-optional-state');
      return el && (el as unknown as { $ready?: boolean }).$ready === true;
    });
  });

  test('hydrates template-only bindings without public observables', async ({ page }) => {
    await expect(page.locator('test-optional-state .heading')).toHaveText('Server heading');
    await expect(page.locator('test-optional-state .count')).toHaveText('Count: 2');
    await expect(page.locator('test-optional-state .selected')).toHaveText('Selected: off');
    await expect(page.locator('test-optional-state .details-link')).toHaveAttribute('href', '/items/42');
    await expect(page.locator('test-optional-state .item')).toHaveText(['Ada', 'Grace']);
    await expect(page.locator('test-optional-state .details')).toHaveCount(0);

    const exposesTemplateOnlyState = await page.evaluate(() => {
      const host = document.querySelector('test-optional-state') as unknown as Record<string, unknown>;
      return Object.prototype.hasOwnProperty.call(host, 'heading') ||
        Object.prototype.hasOwnProperty.call(host, 'items') ||
        Object.prototype.hasOwnProperty.call(host, 'showDetails');
    });
    expect(exposesTemplateOnlyState).toBe(false);
  });

  test('updates omitted template bindings through setState', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-optional-state') as {
        setState(state: Record<string, unknown>): void;
      } | null;
      host?.setState({
        heading: 'Updated heading',
        count: 3,
        selectedId: '99',
        items: [
          { name: 'Linus' },
          { name: 'Radia' },
          { name: 'Margaret' },
        ],
        showDetails: true,
        details: 'Loaded from hidden state',
      });
    });

    await expect(page.locator('test-optional-state .heading')).toHaveText('Updated heading');
    await expect(page.locator('test-optional-state .count')).toHaveText('Count: 3');
    await expect(page.locator('test-optional-state .details-link')).toHaveAttribute('href', '/items/99');
    await expect(page.locator('test-optional-state .item')).toHaveText(['Linus', 'Radia', 'Margaret']);
    await expect(page.locator('test-optional-state .details')).toHaveText('Loaded from hidden state');
  });

  test('updates omitted template bindings when host attributes change', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-optional-state');
      host?.setAttribute('heading', 'Attribute heading');
      host?.setAttribute('selected-id', '123');
    });

    await expect(page.locator('test-optional-state .heading')).toHaveText('Attribute heading');
    await expect(page.locator('test-optional-state .details-link')).toHaveAttribute('href', '/items/123');
  });

  test('keeps observables for state used by component code', async ({ page }) => {
    await page.locator('test-optional-state .toggle').click();
    await expect(page.locator('test-optional-state .selected')).toHaveText('Selected: on');
  });
});
