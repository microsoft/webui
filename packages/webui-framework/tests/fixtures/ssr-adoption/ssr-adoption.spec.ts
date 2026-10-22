// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test('adopts SSR without parsing client templates, then materializes new content lazily', async ({ page }) => {
  await page.addInitScript(() => {
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, 'innerHTML')!;
    const captures: string[] = [];
    Object.defineProperty(window, '__templateParses', { value: captures });
    Object.defineProperty(Element.prototype, 'innerHTML', {
      ...descriptor,
      set(value: string) {
        if (this instanceof HTMLTemplateElement) captures.push(String(value));
        descriptor.set!.call(this, value);
      },
    });
  });
  await page.goto('/ssr-adoption/fixture.html');
  const library = page.locator('adoption-library');
  await expect(library.locator('adoption-project')).toHaveCount(2);
  await expect(library.locator('adoption-status').first()).toContainText('Ready');
  const parses = () => page.evaluate(() => Reflect.get(window, '__templateParses') as string[]);
  expect(await parses()).toEqual([]);

  await library.getByRole('button', { name: 'Toggle details' }).click();
  await expect(library.locator('aside')).toHaveCount(0);
  expect(await parses()).toEqual([]);
  await library.getByRole('button', { name: 'Toggle details' }).click();
  await expect(library.locator('aside strong')).toHaveText('Projects are stored locally.');
  expect((await parses()).length).toBeGreaterThan(0);

  await library.getByRole('button', { name: 'Reverse projects' }).click();
  await expect(library.locator('adoption-project h2').first()).toHaveText('Performance');
  await library.getByRole('button', { name: 'Add project' }).click();
  await expect(library.locator('adoption-project')).toHaveCount(3);
  await expect(library.locator('adoption-project').last().locator('h2')).toHaveText('New project');
  await library.locator('adoption-project').last().getByRole('button', { name: 'Toggle notes' }).click();
  await expect(library.locator('adoption-project').last().locator('.notes')).toContainText('New project');
});
