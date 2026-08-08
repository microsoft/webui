// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test('scopes Light DOM policy CSS to the nested component host', async ({ page }) => {
  await page.goto('/lazy-hydration-light-shadow-policy/fixture.html');

  const styles = await page.evaluate(() => {
    const parent = document.querySelector('#light-shadow-parent');
    const shadowRoot = parent?.shadowRoot;
    const offscreen = shadowRoot?.querySelector('#light-shadow-offscreen');
    const eager = shadowRoot?.querySelector('#light-shadow-eager');
    if (!parent || !offscreen || !eager) {
      throw new Error('mixed DOM policy fixture is incomplete');
    }
    const parentStyle = getComputedStyle(parent);
    const offscreenStyle = getComputedStyle(offscreen);
    const eagerStyle = getComputedStyle(eager);
    return {
      parent: [
        parentStyle.contentVisibility,
        parentStyle.containIntrinsicBlockSize,
      ],
      offscreen: [
        offscreenStyle.contentVisibility,
        offscreenStyle.containIntrinsicBlockSize,
      ],
      eager: eagerStyle.contentVisibility,
    };
  });

  expect(styles).toEqual({
    parent: ['visible', 'none'],
    offscreen: ['auto', 'auto 72px'],
    eager: 'visible',
  });
});
