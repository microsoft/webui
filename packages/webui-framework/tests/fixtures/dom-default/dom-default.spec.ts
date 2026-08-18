// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test('unwrapped components default to Shadow DOM and support slots', async ({ page }) => {
  await page.goto('/dom-default/fixture.html');
  const host = page.locator('#default-shadow');

  await expect(host).toHaveJSProperty('$ready', true);
  const result = await host.evaluate((element) => {
    const root = element.shadowRoot;
    const slot = root?.querySelector('slot');
    const label = root?.querySelector('.label');
    return {
      hasShadow: !!root,
      lightMarker: element.hasAttribute('data-wl'),
      label: label?.textContent,
      labelColor: label instanceof HTMLElement ? getComputedStyle(label).color : null,
      projected: slot instanceof HTMLSlotElement
        ? slot.assignedElements().map(child => child.textContent?.trim())
        : [],
      resources: root
        ? Array.from(root.children)
          .filter(child => child.hasAttribute('data-webui-resource'))
          .map(child => child.getAttribute('data-webui-resource'))
        : [],
    };
  });

  expect(result).toEqual({
    hasShadow: true,
    lightMarker: false,
    label: 'Default shadow',
    labelColor: 'rgb(12, 34, 56)',
    projected: ['Projected content'],
    resources: ['test-shadow-default'],
  });
  await expect(host.locator(':scope > .projected')).toHaveCSS('color', 'rgb(128, 0, 128)');

  const reconnected = await host.evaluate((element) => {
    const parent = element.parentElement;
    if (!parent) throw new Error('default Shadow host has no parent');
    element.remove();
    parent.appendChild(element);
    const root = element.shadowRoot;
    const label = root?.querySelector('.label');
    return {
      detail: root?.querySelector('.detail')?.textContent?.trim(),
      labelColor: label instanceof HTMLElement ? getComputedStyle(label).color : null,
      resources: root
        ? Array.from(root.children)
          .filter(child => child.hasAttribute('data-webui-resource'))
          .map(child => child.getAttribute('data-webui-resource'))
        : [],
    };
  });
  expect(reconnected).toEqual({
    detail: 'Structural detail',
    labelColor: 'rgb(12, 34, 56)',
    resources: ['test-shadow-default'],
  });
});
