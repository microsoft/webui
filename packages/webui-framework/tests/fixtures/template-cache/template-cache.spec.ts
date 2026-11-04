// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import type {} from './element.js';

test('collection before dormant-host registration preserves a hidden dependency', async ({ page }) => {
  await page.goto('/template-cache/fixture.html');
  await page.waitForFunction(() => !!customElements.get('cache-child'));
  expect(await page.evaluate(() => ({
    released: window.__templateCacheFixture.initiallyReleased,
    cached: Object.keys(window.__webui?.templates ?? {}).length,
  }))).toEqual({ released: 3, cached: 0 });

  const session = await page.context().newCDPSession(page);
  await session.send('HeapProfiler.collectGarbage');
  await page.getByRole('button', { name: 'Show child' }).click();
  await expect(page.locator('cache-child p')).toHaveText('Dependency survived collection');
  expect(await page.evaluate(() => Object.keys(window.__webui?.templates ?? {}).length)).toBe(0);
  await session.detach();
});

test('reports unused-template retention after disconnect without assuming immediate GC', async ({ page }) => {
  await page.goto('/template-cache/fixture.html');
  await page.waitForFunction(() => !!customElements.get('cache-unused'));
  const session = await page.context().newCDPSession(page);
  await session.send('HeapProfiler.collectGarbage');
  const before = await session.send('Runtime.getHeapUsage');

  await page.evaluate(() => {
    document.querySelector('cache-unused')?.remove();
    window.__templateCacheFixture.collect();
  });
  let retained = true;
  for (let attempt = 0; attempt < 3 && retained; attempt++) {
    await page.evaluate(() => new Promise<void>(resolve => setTimeout(resolve, 0)));
    await session.send('HeapProfiler.collectGarbage');
    retained = await page.evaluate(() => window.__templateCacheFixture.unused.deref() !== undefined);
  }
  const after = await session.send('Runtime.getHeapUsage');
  console.log(JSON.stringify({
    templateCacheMemory: {
      unusedTemplateRetained: retained,
      beforeUsedHeap: before.usedSize,
      afterUsedHeap: after.usedSize,
    },
  }));
  await expect(page.locator('cache-unused')).toHaveCount(0);
  expect(await page.evaluate(() => Object.keys(window.__webui?.templates ?? {}).length)).toBe(0);
  await session.detach();
});
