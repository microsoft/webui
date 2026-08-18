// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('light-dom pipeline', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/light-dom/fixture.html');
    await expect(page.locator('#light-root')).toHaveJSProperty('$ready', true);
    await expect(page.locator('#shadow-opt-in')).toHaveJSProperty('$ready', true);
  });

  test('unwrapped component uses Light DOM and applies scoped CSS', async ({ page }) => {
    await expect(page.locator('test-light-dom .greeting')).toHaveText('Hello');
    await expect(page.locator('test-light-dom .name')).toHaveText('World');
    await expect(page.locator('test-light-dom .greeting')).toHaveCSS(
      'color',
      'rgb(12, 34, 56)',
    );
    await expect(page.locator('test-light-dom .greeting')).toHaveCSS(
      'background-color',
      'rgba(0, 0, 0, 0)',
    );
    await expect(page.locator('test-light-dom .child-boundary')).toHaveCSS(
      'border-top-width',
      '0px',
    );

    const result = await page.locator('#light-root').evaluate((host) => {
      const marker = Array.from(
        host.querySelector('.greeting')?.attributes ?? [],
        attribute => attribute.name,
      ).find(name => name.startsWith('data-wl-'));
      return {
        hasShadow: !!host.shadowRoot,
        lightMarker: host.hasAttribute('data-wl'),
        // The host owns `data-wl`; only its declared content is stamped.
        hostUnstamped: !marker || !host.hasAttribute(marker),
        scopedCss: !!marker && Array.from(
          document.head.querySelectorAll('style[data-webui-resource]'),
          style => style.textContent,
        ).some(css => css?.includes(`:where([${marker}])`)),
      };
    });
    expect(result).toEqual({
      hasShadow: false,
      lightMarker: true,
      hostUnstamped: true,
      scopedCss: true,
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

  test('a parent may style a nested Light host but never its internals', async ({ page }) => {
    const child = page.locator('test-light-child').first();

    // The nested host is declared by the parent template, so it carries the
    // parent's marker and the parent's `test-light-child` rule applies.
    await expect(child).toHaveCSS('outline-color', 'rgb(1, 2, 3)');
    await expect(child).toHaveCSS('display', 'block');

    // `.child-label` is declared by the child template, so it carries only the
    // child's marker. The parent's identically-named rule must not reach it.
    await expect(child.locator('.child-label')).toHaveCSS(
      'color',
      'rgb(34, 139, 34)',
    );

    const markers = await child.evaluate((host) => {
      const label = host.querySelector('.child-label');
      const names = (element: Element | null) => (element
        ? element.getAttributeNames().filter(name => name.startsWith('data-wl-'))
        : []);
      return { host: names(host), label: names(label) };
    });

    expect(markers.host).toHaveLength(1);
    expect(markers.label).toHaveLength(1);
    expect(markers.host).not.toEqual(markers.label);
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
        nestedLightMarkers: root?.querySelectorAll(
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
      nestedLightMarkers: 2,
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
