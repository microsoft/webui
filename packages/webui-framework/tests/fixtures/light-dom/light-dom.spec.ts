// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('light-dom pipeline', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/light-dom/fixture.html');
    await expect(page.locator('#light-root')).toHaveJSProperty('$ready', true);
    await expect(page.locator('#shadow-opt-in')).toHaveJSProperty('$ready', true);
  });

  test('unwrapped component uses Light DOM and applies global CSS', async ({ page }) => {
    await expect(page.locator('test-light-dom .greeting')).toHaveText('Hello');
    await expect(page.locator('test-light-dom .name')).toHaveText('World');
    await expect(page.locator('test-light-dom .greeting')).toHaveCSS(
      'color',
      'rgb(12, 34, 56)',
    );
    await expect(page.locator('test-light-dom .greeting')).toHaveCSS(
      'background-color',
      'rgb(255, 0, 0)',
    );
    await expect(page.locator('test-light-dom .child-boundary')).toHaveCSS(
      'border-top-width',
      '5px',
    );

    const result = await page.locator('#light-root').evaluate((host) => {
      return {
        hasShadow: !!host.shadowRoot,
        hasLightMarker: host.hasAttribute('data-wl'),
        globalCss: Array.from(
          document.head.querySelectorAll('style[data-webui-resource]'),
          style => style.textContent,
        ).some(css => css?.includes('.greeting')),
      };
    });
    expect(result).toEqual({
      hasShadow: false,
      hasLightMarker: false,
      globalCss: true,
    });
  });

  test('installs the Light parent/child closure in order and deduplicates client mounts', async ({ page }) => {
    const resourceIds = () => page.locator(
      'head > [data-webui-resource]',
    ).evaluateAll(elements => elements.map(
      element => element.getAttribute('data-webui-resource'),
    ));

    expect(await resourceIds()).toEqual(['test-light-dom', 'test-light-child']);
    await expect(page.locator('#light-root > .child-boundary > test-light-child')).toHaveCount(2);

    await page.locator('#light-root').evaluate((host) => {
      const spawnChild = Reflect.get(host, 'spawnChild');
      if (typeof spawnChild !== 'function') {
        throw new Error('Missing spawnChild()');
      }
      Reflect.apply(spawnChild, host, []);
    });

    await expect(page.locator('.client-children > test-light-child')).toHaveCount(1);
    await expect(page.locator('.client-children .child-label')).toHaveCSS(
      'color',
      'rgb(34, 139, 34)',
    );
    expect(await resourceIds()).toEqual(['test-light-dom', 'test-light-child']);
  });

  test('global Light CSS reaches nested Light hosts and internals', async ({ page }) => {
    const child = page.locator('test-light-child').first();

    // The nested host is declared by the parent template, and global CSS
    // applies its host rule normally.
    await expect(child).toHaveCSS('outline-color', 'rgb(1, 2, 3)');
    await expect(child).toHaveCSS('display', 'block');

    // The parent and child both use `.child-label`; the child stylesheet is
    // later in the closure, so its rule wins by normal cascade order.
    await expect(child.locator('.child-label')).toHaveCSS(
      'color',
      'rgb(34, 139, 34)',
    );

    await expect(child).not.toHaveAttribute('data-wl');
    const markerNames = await child.evaluate((host) => [
      ...host.getAttributeNames(),
      ...(host.querySelector('.child-label')?.getAttributeNames() ?? []),
    ].filter(name => name.startsWith('data-wl')));
    expect(markerNames).toEqual([]);
  });

  test('Shadow opt-in cuts the Document closure and projects a native slot', async ({ page }) => {
    const result = await page.locator('#shadow-opt-in').evaluate((host) => {
      const root = host.shadowRoot;
      const slot = root?.querySelector('slot');
      return {
        hasShadow: !!root,
        lightMarker: host.hasAttribute('data-wl'),
        projected: slot instanceof HTMLSlotElement
          ? slot.assignedElements().map(element => element.textContent?.trim())
          : [],
        resourceIds: root
          ? Array.from(root.children)
            .filter(element => element.hasAttribute('data-webui-resource'))
            .map(element => element.getAttribute('data-webui-resource'))
          : [],
        nestedLightMarkerCount: root?.querySelectorAll(
          'test-shadow-light-child[data-wl]',
        ).length ?? 0,
        nestedColor: (() => {
          const label = root?.querySelector('.nested-label');
          return label instanceof HTMLElement ? getComputedStyle(label).color : null;
        })(),
      };
    });

    expect(result).toEqual({
      hasShadow: true,
      lightMarker: false,
      projected: ['Projected label'],
      resourceIds: ['test-shadow-opt-in', 'test-shadow-light-child'],
      nestedLightMarkerCount: 0,
      nestedColor: 'rgb(70, 130, 180)',
    });
    await expect(page.locator('#shadow-opt-in > .projected')).toHaveCSS(
      'color',
      'rgb(128, 0, 128)',
    );

    const documentResources = await page.locator(
      'head > [data-webui-resource]',
    ).evaluateAll(elements => elements.map(
      element => element.getAttribute('data-webui-resource'),
    ));
    expect(documentResources).not.toContain('test-shadow-opt-in');
    expect(documentResources).not.toContain('test-shadow-light-child');
  });
});
