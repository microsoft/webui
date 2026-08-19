// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

const STYLESHEET_PATH = '/stylesheet-preload/early.css';
const ATTRIBUTE_STYLESHEET_PATH = '/stylesheet-preload/attributes.css';

test.describe('pre-registration stylesheet preload', () => {
  test('reuses one resolved-href preload during registration', async ({ page }) => {
    let requests = 0;
    let releaseStylesheet!: () => void;
    let stylesheetRequested!: () => void;
    const stylesheetGate = new Promise<void>(resolve => {
      releaseStylesheet = resolve;
    });
    const requestSeen = new Promise<void>(resolve => {
      stylesheetRequested = resolve;
    });
    await page.route(`**${STYLESHEET_PATH}`, async route => {
      requests += 1;
      stylesheetRequested();
      await stylesheetGate;
      await route.continue();
    });

    await page.goto('/stylesheet-preload/fixture.html', {
      waitUntil: 'domcontentloaded',
    });
    await page.waitForFunction(() => {
      return typeof window.__registerPreloadedStylesheetFixture === 'function';
    });
    await requestSeen;

    await expect(page.evaluate(() => {
      return Array.from(
        document.head.querySelectorAll<HTMLLinkElement>(
          'link[rel="preload"][as="style"]',
        ),
      ).filter(link => {
        return new URL(link.href).pathname === '/stylesheet-preload/early.css';
      }).length;
    })).resolves.toBe(1);

    await page.evaluate(() => {
      window.__registerPreloadedStylesheetFixture?.();
    });
    await expect.poll(async () => page.evaluate(() => {
      const child = document.querySelector('test-preloaded-stylesheet');
      return {
        links:
          child?.shadowRoot?.querySelectorAll('link[rel="stylesheet"]').length ??
          0,
        preloads: Array.from(
          document.head.querySelectorAll<HTMLLinkElement>(
            'link[rel="preload"][as="style"]',
          ),
        ).filter(link => {
          return new URL(link.href).pathname ===
            '/stylesheet-preload/early.css';
        }).length,
      };
    })).toEqual({ links: 1, preloads: 1 });

    releaseStylesheet();
    await expect.poll(async () => page.evaluate(() => {
      const child = document.querySelector('test-preloaded-stylesheet');
      const label = child?.shadowRoot?.querySelector('.early-label');
      return {
        adopted: child?.shadowRoot?.adoptedStyleSheets.length ?? 0,
        color:
          label instanceof HTMLElement ? getComputedStyle(label).color : null,
        disabled:
          child?.shadowRoot
            ?.querySelector<HTMLLinkElement>('link[rel="stylesheet"]')
            ?.disabled ?? false,
        preloads: Array.from(
          document.head.querySelectorAll<HTMLLinkElement>(
            'link[rel="preload"][as="style"]',
          ),
        ).filter(link => {
          return new URL(link.href).pathname ===
            '/stylesheet-preload/early.css';
        }).length,
      };
    })).toEqual({
      adopted: 1,
      color: 'rgb(21, 83, 45)',
      disabled: true,
      preloads: 0,
    });
    expect(requests).toBe(1);
  });

  test('removes an unclaimed preload after the speculation window', async ({
    page,
  }) => {
    await page.goto('/stylesheet-preload/fixture.html', {
      waitUntil: 'domcontentloaded',
    });
    await page.waitForFunction(() => {
      return typeof window.__registerPreloadedStylesheetFixture === 'function';
    });

    await expect.poll(async () => page.evaluate(() => {
      return Array.from(
        document.head.querySelectorAll<HTMLLinkElement>(
          'link[rel="preload"][as="style"]',
        ),
      ).filter(link => {
        return new URL(link.href).pathname === '/stylesheet-preload/early.css';
      }).length;
    }), { timeout: 5_000 }).toBe(0);
  });

  test('is a no-op without constructable stylesheet support', async ({
    page,
  }) => {
    await page.goto('/stylesheet-preload/fixture.html', {
      waitUntil: 'domcontentloaded',
    });
    await page.waitForFunction(() => {
      return typeof window.__preloadUnsupportedStylesheetFixture === 'function';
    });

    await page.evaluate(() => {
      const replaceSync = CSSStyleSheet.prototype.replaceSync;
      Object.defineProperty(CSSStyleSheet.prototype, 'replaceSync', {
        configurable: true,
        value: undefined,
      });
      try {
        window.__preloadUnsupportedStylesheetFixture?.();
      } finally {
        Object.defineProperty(CSSStyleSheet.prototype, 'replaceSync', {
          configurable: true,
          value: replaceSync,
        });
      }
    });

    await expect(page.evaluate(() => {
      return Array.from(
        document.head.querySelectorAll<HTMLLinkElement>(
          'link[rel="preload"][as="style"]',
        ),
      ).filter(link => {
        return new URL(link.href).pathname ===
          '/stylesheet-preload/unsupported.css';
      }).length;
    })).resolves.toBe(0);
  });

  test('replaces a bare preload when registration attributes differ', async ({
    page,
  }) => {
    let requests = 0;
    let releaseStylesheet!: () => void;
    let stylesheetRequested!: () => void;
    const stylesheetGate = new Promise<void>(resolve => {
      releaseStylesheet = resolve;
    });
    const requestSeen = new Promise<void>(resolve => {
      stylesheetRequested = resolve;
    });
    await page.route(`**${ATTRIBUTE_STYLESHEET_PATH}`, async route => {
      requests += 1;
      if (requests === 2) stylesheetRequested();
      await stylesheetGate;
      await route.continue();
    });

    await page.goto('/stylesheet-preload/fixture.html', {
      waitUntil: 'domcontentloaded',
    });
    await page.waitForFunction(() => {
      return typeof window.__registerAttributeStylesheetFixture === 'function';
    });
    await page.evaluate(() => {
      window.__preloadAttributeStylesheetFixture?.();
    });
    await expect.poll(() => requests).toBe(1);

    await expect(page.evaluate(() => {
      const preload = Array.from(
        document.head.querySelectorAll<HTMLLinkElement>(
          'link[rel="preload"][as="style"]',
        ),
      ).find(link => {
        return new URL(link.href).pathname ===
          '/stylesheet-preload/attributes.css';
      });
      return preload?.getAttribute('crossorigin') ?? null;
    })).resolves.toBeNull();

    await page.evaluate(() => {
      window.__registerAttributeStylesheetFixture?.();
    });
    await requestSeen;
    await expect(page.evaluate(() => {
      const preloads = Array.from(
        document.head.querySelectorAll<HTMLLinkElement>(
          'link[rel="preload"][as="style"]',
        ),
      ).filter(link => {
        return new URL(link.href).pathname ===
          '/stylesheet-preload/attributes.css';
      });
      return preloads.map(preload => ({
        crossOrigin: preload.getAttribute('crossorigin'),
        referrerPolicy: preload.referrerPolicy,
      }));
    })).resolves.toEqual([{
      crossOrigin: 'anonymous',
      referrerPolicy: 'no-referrer',
    }]);

    releaseStylesheet();
    await expect.poll(async () => page.evaluate(() => {
      const child = document.querySelector('test-attribute-stylesheet');
      const label = child?.shadowRoot?.querySelector('.attribute-label');
      return {
        adopted: child?.shadowRoot?.adoptedStyleSheets.length ?? 0,
        color:
          label instanceof HTMLElement ? getComputedStyle(label).color : null,
      };
    })).toEqual({
      adopted: 1,
      color: 'rgb(72, 61, 139)',
    });
    expect(requests).toBe(2);
  });
});
