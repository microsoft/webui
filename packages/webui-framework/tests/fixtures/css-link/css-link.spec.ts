// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';

test.describe('css link fixture', () => {
  async function openFixture(page: Page): Promise<void> {
    await page.goto('/css-link/fixture.html');
    await page.waitForSelector('test-link-host .spawn');
  }

  test('client-created components apply link stylesheet styles', async ({ page }) => {
    await openFixture(page);

    // Host component should have green text from host.css
    await expect.poll(async () => page.locator('test-link-host').evaluate((host) => {
      const label = (host.shadowRoot ?? host).querySelector('.host-label');
      return label instanceof HTMLElement ? getComputedStyle(label).color : null;
    })).toBe('rgb(34, 139, 34)');

    // Spawn a client-created child component
    const relativeAsset = page.waitForRequest(request => {
      return new URL(request.url()).pathname === '/css-link/relative-marker.svg';
    });
    await page.locator('test-link-host .spawn').click();
    await relativeAsset;

    // Child component should have purple text from child.css
    // (styles applied via adoptedStyleSheets or <link>, either is correct)
    await expect.poll(async () => page.locator('test-link-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      const label = (child?.shadowRoot ?? child)?.querySelector('.child-label');
      return label instanceof HTMLElement ? getComputedStyle(label).color : null;
    })).toBe('rgb(128, 0, 128)');

    await expect(page.locator('test-link-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      return {
        adopted: child?.shadowRoot?.adoptedStyleSheets.length ?? 0,
        disabled:
          Array.from(
            child?.shadowRoot?.querySelectorAll('link[rel~="stylesheet"]') ?? [],
          ).filter(link => (link as HTMLLinkElement).disabled).length,
        links: child?.shadowRoot?.querySelectorAll('link[rel~="stylesheet"]').length ?? 0,
      };
    })).resolves.toEqual({ adopted: 2, disabled: 2, links: 2 });

    await page.locator('test-link-host').evaluate(async (host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-link-child') as
        | (HTMLElement & { message?: string })
        | null;
      child?.remove();
      await Promise.resolve();
      if (child) document.body.appendChild(child);
      if (child) child.message = 'after reconnect';
    });
    await expect.poll(async () => page.locator('test-link-child').evaluate(child => {
      return child.shadowRoot?.querySelector('.child-label')?.textContent ?? null;
    })).toBe('Child after reconnect');
  });

  test('reuses an attribute-matched preload and removes it after native load', async ({
    page,
  }) => {
    let requests = 0;
    await page.route('**/attributes.css', async route => {
      requests += 1;
      await route.fulfill({
        body: '.attribute-label { color: rgb(12, 34, 56); }\n',
        contentType: 'text/css',
        status: 200,
      });
    });
    await openFixture(page);

    await expect(page.evaluate(() => {
      const preload = Array.from(
        document.head.querySelectorAll<HTMLLinkElement>(
          'link[rel="preload"][as="style"]',
        ),
      ).find(link => new URL(link.href).pathname === '/css-link/attributes.css');
      return preload
        ? {
          as: preload.as,
          crossOrigin: preload.getAttribute('crossorigin'),
          integrity: preload.integrity,
          referrerPolicy: preload.referrerPolicy,
        }
        : null;
    })).resolves.toEqual({
      as: 'style',
      crossOrigin: 'anonymous',
      integrity: 'sha256-vADsiNbfyGjSNTts/BjSlWmFqX43h0n5YRSt1uYYo6U=',
      referrerPolicy: 'no-referrer',
    });

    await page.evaluate(() => {
      document.body.appendChild(
        document.createElement('test-link-attributes-child'),
      );
    });
    await expect.poll(async () => page.evaluate(() => {
      const child = document.querySelector('test-link-attributes-child');
      const label = child?.shadowRoot?.querySelector('.attribute-label');
      const preloads = Array.from(
        document.head.querySelectorAll<HTMLLinkElement>(
          'link[rel="preload"][as="style"]',
        ),
      ).filter(link => {
        return new URL(link.href).pathname === '/css-link/attributes.css';
      }).length;
      return {
        adopted: child?.shadowRoot?.adoptedStyleSheets.length ?? 0,
        color:
          label instanceof HTMLElement ? getComputedStyle(label).color : null,
        preloads,
      };
    })).toEqual({
      adopted: 1,
      color: 'rgb(12, 34, 56)',
      preloads: 0,
    });
    expect(requests).toBe(1);
  });

  test('preserves cascade order for authored and lifecycle style elements', async ({
    page,
  }) => {
    await openFixture(page);
    const warmSource = await page.evaluate(async () => {
      const styleBlock = document.createElement('test-style-block-child');
      const warmSource = document.createElement('test-lifecycle-style-child');
      document.body.append(styleBlock, warmSource);
      const link = warmSource.shadowRoot?.querySelector('link');
      if (link && !link.sheet) {
        await new Promise<void>(resolve => {
          link.addEventListener('load', () => resolve(), { once: true });
          link.addEventListener('error', () => resolve(), { once: true });
        });
      }
      await new Promise<void>(resolve =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
      );
      const lifecycleStyle = document.createElement('test-lifecycle-style-child');
      lifecycleStyle.setAttribute('data-add-style', '');
      document.body.append(lifecycleStyle);
      return {
        adopted: warmSource.shadowRoot?.adoptedStyleSheets.length ?? -1,
        disabled: (link as HTMLLinkElement | null)?.disabled ?? false,
      };
    });
    expect(warmSource).toEqual({ adopted: 1, disabled: true });

    await expect.poll(async () => {
      return await page.evaluate(() => {
        return [
          'test-style-block-child',
          'test-lifecycle-style-child:last-of-type',
        ].map(selector => {
          const child = document.querySelector(selector);
          const label = child?.shadowRoot?.querySelector('.child-label');
          return {
            adopted: child?.shadowRoot?.adoptedStyleSheets.length ?? -1,
            color:
              label instanceof HTMLElement
                ? getComputedStyle(label).color
                : null,
            links:
              child?.shadowRoot?.querySelectorAll('link[rel="stylesheet"]')
                .length ?? -1,
          };
        });
      });
    }).toEqual([
      { adopted: 0, color: 'rgb(0, 128, 0)', links: 1 },
      { adopted: 0, color: 'rgb(255, 0, 0)', links: 1 },
    ]);
  });

  test('constructs from native CSSOM without a generic fetch warmup', async ({
    page,
  }) => {
    let genericFetches = 0;
    page.on('request', request => {
      if (
        new URL(request.url()).pathname === '/css-link/child.css' &&
        request.resourceType() === 'fetch'
      ) {
        genericFetches += 1;
      }
    });
    await openFixture(page);

    await page.locator('test-link-host .spawn').click();
    await expect.poll(async () => page.locator('test-link-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      const label = child?.shadowRoot?.querySelector('.child-label');
      return {
        adopted: child?.shadowRoot?.adoptedStyleSheets.length ?? 0,
        color: label instanceof HTMLElement ? getComputedStyle(label).color : null,
      };
    })).toEqual({ adopted: 2, color: 'rgb(128, 0, 128)' });
    expect(genericFetches).toBe(0);
  });

  test('keeps service-worker-backed stylesheets native', async ({ page }) => {
    await page.goto('/css-link/fixture.html');
    await page.evaluate(async () => {
      await navigator.serviceWorker.register('/css-link/sw-css.js');
      await navigator.serviceWorker.ready;
    });
    await page.reload();
    await page.waitForFunction(() => navigator.serviceWorker.controller !== null);
    await page.waitForSelector('test-link-host .spawn');

    await page.locator('test-link-host .spawn').click();
    await expect.poll(async () => page.locator('test-link-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      const label = child?.shadowRoot?.querySelector('.child-label');
      return {
        adopted: child?.shadowRoot?.adoptedStyleSheets.length ?? -1,
        color:
          label instanceof HTMLElement ? getComputedStyle(label).color : null,
        links:
          child?.shadowRoot?.querySelectorAll('link[rel="stylesheet"]').length ??
          -1,
      };
    })).toEqual({
      adopted: 0,
      color: 'rgb(128, 0, 128)',
      links: 2,
    });
  });

  test('never exposes an unstyled frame while construction is pending', async ({ page }) => {
    await page.addInitScript(() => {
      const original = CSSStyleSheet.prototype.replaceSync;
      const visibilityDuringPromotion: string[] = [];
      Object.defineProperty(window, '__webuiPromotionVisibility', {
        configurable: true,
        value: visibilityDuringPromotion,
      });
      Object.defineProperty(CSSStyleSheet.prototype, 'replaceSync', {
        configurable: true,
        value(this: CSSStyleSheet, cssText: string): void {
          const host = document.querySelector('test-link-host');
          const child = (host?.shadowRoot ?? host)?.querySelector('test-link-child');
          if (child instanceof HTMLElement) {
            visibilityDuringPromotion.push(
              child.style.getPropertyValue('visibility'),
            );
          }
          original.call(this, cssText);
        },
      });
    });
    let releaseLinks!: () => void;
    let cssRequested!: () => void;
    const linkGate = new Promise<void>(resolve => {
      releaseLinks = resolve;
    });
    const requestSeen = new Promise<void>(resolve => {
      cssRequested = resolve;
    });
    await page.route('**/child.css', async route => {
      cssRequested();
      await linkGate;
      await route.continue();
    });
    await page.route('**/styles/relative.css', async route => {
      await linkGate;
      await route.continue();
    });
    await openFixture(page);

    await page.locator('test-link-host .spawn').click();
    await requestSeen;
    const hidden = await page.locator('test-link-host').evaluate(async (host) => {
      await new Promise<void>(resolve =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
      );
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      return child ? getComputedStyle(child).visibility : null;
    });
    expect(hidden).toBe('hidden');

    await installFrameObserver(page);
    releaseLinks();
    const result = await readFrameObserver(page);
    expect(result.exposed).toBe(false);
    expect(result.timedOut).toBe(false);
    expect(result.adopted).toBe(2);
    expect(result.links).toBe(2);
    const visibilityDuringPromotion = await page.evaluate(() => {
      return (
        window as typeof window & { __webuiPromotionVisibility?: string[] }
      ).__webuiPromotionVisibility;
    });
    expect(visibilityDuringPromotion?.length).toBeGreaterThan(0);
    expect(visibilityDuringPromotion?.every(value => value === '')).toBe(true);
  });

  test('keeps the guard isolated from authored cascade layers', async ({ page }) => {
    let releaseRelative!: () => void;
    const relativeGate = new Promise<void>(resolve => {
      releaseRelative = resolve;
    });
    await page.route('**/child.css', async route => {
      await route.fulfill({
        body:
          '@layer webui-prepaint-guard { :host { visibility: visible !important; } }' +
          '.child-label { color: rgb(128, 0, 128); }',
        contentType: 'text/css',
        status: 200,
      });
    });
    await page.route('**/styles/relative.css', async route => {
      if (route.request().resourceType() === 'stylesheet') {
        await relativeGate;
      }
      await route.continue();
    });
    await openFixture(page);

    await page.locator('test-link-host .spawn').click();
    await expect.poll(async () => page.locator('test-link-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      return !!child?.shadowRoot?.querySelector('link')?.sheet;
    })).toBe(true);
    await expect(page.locator('test-link-host').evaluate(async (host) => {
      await new Promise<void>(resolve =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
      );
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      return child ? getComputedStyle(child).visibility : null;
    })).resolves.toBe('hidden');

    releaseRelative();
    await expect.poll(async () => page.locator('test-link-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      return child?.shadowRoot?.adoptedStyleSheets.length ?? 0;
    })).toBe(2);
  });

  test('cancels host visibility transitions while styles are pending', async ({ page }) => {
    let releaseLinks!: () => void;
    let linkRequested!: () => void;
    const linkGate = new Promise<void>(resolve => {
      releaseLinks = resolve;
    });
    const requestSeen = new Promise<void>(resolve => {
      linkRequested = resolve;
    });
    await page.route('**/*.css', async route => {
      if (
        route.request().resourceType() === 'stylesheet' &&
        (route.request().url().endsWith('/child.css') ||
          route.request().url().endsWith('/styles/relative.css'))
      ) {
        linkRequested();
        await linkGate;
      }
      await route.continue();
    });
    await openFixture(page);
    await page.locator('test-link-host').evaluate((host) => {
      const child = document.createElement('test-link-child');
      child.style.setProperty('visibility', 'visible', 'important');
      child.style.setProperty(
        'transition-property',
        'visibility',
        'important',
      );
      child.style.setProperty('transition-duration', '10s');
      (host.shadowRoot ?? host).querySelector('.slot')?.appendChild(child);
    });

    await requestSeen;
    await expect(page.locator('test-link-host').evaluate(async (host) => {
      await new Promise<void>(resolve =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
      );
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      const style = child ? getComputedStyle(child) : null;
      return {
        transition: style?.transitionProperty ?? null,
        visibility: style?.visibility ?? null,
      };
    })).resolves.toEqual({ transition: 'none', visibility: 'hidden' });

    releaseLinks();
    await expect.poll(async () => page.locator('test-link-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector(
        'test-link-child',
      ) as HTMLElement | null;
      return {
        duration: child?.style.getPropertyValue('transition-duration') ?? null,
        priority:
          child?.style.getPropertyPriority('transition-property') ?? null,
        transition:
          child?.style.getPropertyValue('transition-property') ?? null,
        visibility: child?.style.getPropertyValue('visibility') ?? null,
      };
    })).toEqual({
      duration: '10s',
      priority: 'important',
      transition: 'visibility',
      visibility: 'visible',
    });
  });

  test('gates the original link fallback until it loads', async ({ page }) => {
    await page.addInitScript(() => {
      Object.defineProperty(CSSStyleSheet.prototype, 'replaceSync', {
        configurable: true,
        value: undefined,
      });
    });
    let releaseLink!: () => void;
    let linkRequested!: () => void;
    const linkGate = new Promise<void>(resolve => {
      releaseLink = resolve;
    });
    const requestSeen = new Promise<void>(resolve => {
      linkRequested = resolve;
    });
    await page.route('**/child.css', async route => {
      linkRequested();
      await linkGate;
      await route.continue();
    });
    await openFixture(page);

    await page.locator('test-link-host .spawn').click();
    await requestSeen;
    const hidden = await page.locator('test-link-host').evaluate(async (host) => {
      await new Promise<void>(resolve =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
      );
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      return child ? getComputedStyle(child).visibility : null;
    });
    expect(hidden).toBe('hidden');

    await installFrameObserver(page);
    releaseLink();
    const result = await readFrameObserver(page);
    expect(result.exposed).toBe(false);
    expect(result.timedOut).toBe(false);
    expect(result.adopted).toBe(0);
    expect(result.links).toBe(2);
  });

  test('does not adopt CSS rejected by native MIME enforcement', async ({ page }) => {
    await page.route('**/child.css', async route => {
      await route.fulfill({
        body: '.child-label { color: rgb(128, 0, 128); }',
        headers: {
          'content-type': 'text/html',
          'x-content-type-options': 'nosniff',
        },
        status: 200,
      });
    });
    const failed = page.waitForEvent('console', message => {
      return message.type() === 'error' &&
        message.text().includes('[WebUI] Stylesheet') &&
        message.text().includes('failed to load');
    });
    await openFixture(page);

    await page.locator('test-link-host .spawn').click();
    await failed;

    await expect(page.locator('test-link-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      return {
        adopted: child?.shadowRoot?.adoptedStyleSheets.length ?? 0,
        links: child?.shadowRoot?.querySelectorAll('link[rel~="stylesheet"]').length ?? 0,
        visibility: child ? getComputedStyle(child).visibility : null,
      };
    })).resolves.toEqual({
      adopted: 0,
      links: 2,
      visibility: 'hidden',
    });
  });

  test('keeps content detached when CSP blocks the shadow guard', async ({ page }) => {
    await page.route('**/css-link/fixture.html', async route => {
      const response = await route.fetch();
      await route.fulfill({
        response,
        headers: {
          ...response.headers(),
          'content-security-policy':
            "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'",
        },
      });
    });
    let releaseLink!: () => void;
    let linkRequested!: () => void;
    const linkGate = new Promise<void>(resolve => {
      releaseLink = resolve;
    });
    const requestSeen = new Promise<void>(resolve => {
      linkRequested = resolve;
    });
    await page.route('**/child.css', async route => {
      linkRequested();
      await linkGate;
      await route.continue();
    });
    await openFixture(page);

    await page.locator('test-link-host .spawn').click();
    await requestSeen;
    await page.locator('test-link-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-link-child') as
        | (HTMLElement & { message?: string })
        | null;
      if (child) child.message = 'updated';
    });
    await expect(page.locator('test-link-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      return {
        hasContent: child?.shadowRoot?.querySelector('.child-label') !== null,
        hydratedCount:
          (child as HTMLElement & { hydratedCount?: number } | null)
            ?.hydratedCount ?? 0,
        links: child?.shadowRoot?.querySelectorAll('link[rel~="stylesheet"]').length ?? 0,
        ready:
          (child as HTMLElement & { $ready?: boolean } | null)?.$ready ?? false,
      };
    })).resolves.toEqual({
      hasContent: false,
      hydratedCount: 0,
      links: 2,
      ready: false,
    });

    releaseLink();
    await expect.poll(async () => {
      return page.locator('test-link-host').evaluate((host) => {
        const child = (host.shadowRoot ?? host).querySelector('test-link-child');
        const label = child?.shadowRoot?.querySelector('.child-label');
        const instance = child as HTMLElement & {
          $ready?: boolean;
          hydratedCount?: number;
          hydratedHadContent?: boolean;
        } | null;
        return {
          color: label instanceof HTMLElement ? getComputedStyle(label).color : null,
          hydratedCount: instance?.hydratedCount ?? 0,
          hydratedHadContent: instance?.hydratedHadContent ?? false,
          ready: instance?.$ready ?? false,
          text: label?.textContent ?? null,
        };
      });
    }).toEqual({
      color: 'rgb(128, 0, 128)',
      hydratedCount: 1,
      hydratedHadContent: true,
      ready: true,
      text: 'Child updated',
    });
  });

  test('waits again when deferred reconciliation changes a bound href', async ({
    page,
  }) => {
    await page.route('**/css-link/fixture.html', async route => {
      const response = await route.fetch();
      await route.fulfill({
        response,
        headers: {
          ...response.headers(),
          'content-security-policy':
            "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'",
        },
      });
    });
    let releaseInitial!: () => void;
    let releaseReplacement!: () => void;
    let initialRequested!: () => void;
    let replacementRequested!: () => void;
    const initialGate = new Promise<void>(resolve => {
      releaseInitial = resolve;
    });
    const replacementGate = new Promise<void>(resolve => {
      releaseReplacement = resolve;
    });
    const initialSeen = new Promise<void>(resolve => {
      initialRequested = resolve;
    });
    const replacementSeen = new Promise<void>(resolve => {
      replacementRequested = resolve;
    });
    await page.route('**/child.css', async route => {
      if (route.request().resourceType() === 'stylesheet') {
        initialRequested();
        await initialGate;
      }
      await route.continue();
    });
    await page.route('**/alternate.css', async route => {
      if (route.request().resourceType() === 'stylesheet') {
        replacementRequested();
        await replacementGate;
      }
      await route.continue();
    });
    await openFixture(page);

    await page.evaluate(() => {
      document.body.appendChild(
        document.createElement('test-link-dynamic-child'),
      );
    });
    await initialSeen;
    await page.locator('test-link-dynamic-child').evaluate(child => {
      const instance = child as HTMLElement & {
        href?: string;
        media?: string;
      };
      instance.href = 'alternate.css';
      instance.media = 'all';
    });
    releaseInitial();
    await replacementSeen;
    await expect(page.locator('test-link-dynamic-child').evaluate(async child => {
      await new Promise<void>(resolve =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
      );
      return {
        content: child.shadowRoot?.querySelector('.child-label') !== null,
        ready:
          (child as HTMLElement & { $ready?: boolean }).$ready ?? false,
      };
    })).resolves.toEqual({ content: false, ready: false });

    releaseReplacement();
    await expect.poll(async () => page.locator('test-link-dynamic-child').evaluate(child => {
      const label = child.shadowRoot?.querySelector('.child-label');
      return {
        color:
          label instanceof HTMLElement ? getComputedStyle(label).color : null,
        ready:
          (child as HTMLElement & { $ready?: boolean }).$ready ?? false,
      };
    })).toEqual({ color: 'rgb(0, 0, 255)', ready: true });
  });

  test('cancels a deferred mount before reconnecting the same element', async ({ page }) => {
    await page.route('**/css-link/fixture.html', async route => {
      const response = await route.fetch();
      await route.fulfill({
        response,
        headers: {
          ...response.headers(),
          'content-security-policy':
            "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'",
        },
      });
    });
    let releaseLink!: () => void;
    let linkRequested!: () => void;
    const linkGate = new Promise<void>(resolve => {
      releaseLink = resolve;
    });
    const requestSeen = new Promise<void>(resolve => {
      linkRequested = resolve;
    });
    await page.route('**/child.css', async route => {
      linkRequested();
      await linkGate;
      await route.continue();
    });
    await openFixture(page);

    await page.evaluate(() => {
      const child = document.createElement('test-link-child');
      child.id = 'reconnected-link-child';
      document.body.appendChild(child);
    });
    await requestSeen;
    await page.evaluate(async () => {
      const child = document.querySelector('#reconnected-link-child') as
        | (HTMLElement & { message?: string })
        | null;
      if (child) child.message = 'reconnected';
      child?.remove();
      await Promise.resolve();
      if (child) document.body.appendChild(child);
    });
    await expect.poll(async () => page.evaluate(() => {
      const child = document.querySelector('#reconnected-link-child');
      return {
        content: child?.shadowRoot?.querySelectorAll('.child-label').length ?? 0,
        links: child?.shadowRoot?.querySelectorAll('link[rel~="stylesheet"]').length ?? 0,
      };
    })).toEqual({ content: 0, links: 2 });

    releaseLink();
    await expect.poll(async () => page.evaluate(() => {
      const child = document.querySelector('#reconnected-link-child') as
        | (HTMLElement & {
          $root?: { nodes: Node[] };
          hydratedCount?: number;
        })
        | null;
      const label = child?.shadowRoot?.querySelector('.child-label');
      return {
        content: child?.shadowRoot?.querySelectorAll('.child-label').length ?? 0,
        hydratedCount: child?.hydratedCount ?? 0,
        text: label?.textContent ?? null,
        tracked: !!label && child?.$root?.nodes.includes(label),
      };
    })).toEqual({
      content: 1,
      hydratedCount: 1,
      text: 'Child reconnected',
      tracked: true,
    });
  });

  test('keeps dynamically bound stylesheet links native', async ({ page }) => {
    await openFixture(page);

    await page.evaluate(() => {
      const child = document.createElement('test-link-dynamic-child');
      document.body.appendChild(child);
    });
    await expect.poll(async () => page.locator('test-link-dynamic-child').evaluate(child => {
      const link = child.shadowRoot?.querySelector('link');
      return {
        adopted: child.shadowRoot?.adoptedStyleSheets.length ?? 0,
        links: child.shadowRoot?.querySelectorAll('link[rel~="stylesheet"]').length ?? 0,
        media: link?.media ?? null,
        visibility: getComputedStyle(child).visibility,
      };
    })).toEqual({
      adopted: 0,
      links: 3,
      media: 'not all',
      visibility: 'visible',
    });
  });

  test('keeps stylesheets with imports on the native link path', async ({ page }) => {
    await openFixture(page);

    await page.evaluate(() => {
      const child = document.createElement('test-link-import-child');
      document.body.appendChild(child);
    });
    await expect.poll(async () => page.locator('test-link-import-child').evaluate(child => {
      const label = child.shadowRoot?.querySelector('.import-label');
      return {
        adopted: child.shadowRoot?.adoptedStyleSheets.length ?? 0,
        color: label ? getComputedStyle(label).color : null,
        links: child.shadowRoot?.querySelectorAll('link[rel~="stylesheet"]').length ?? 0,
      };
    })).toEqual({
      adopted: 0,
      color: 'rgb(0, 128, 0)',
      links: 1,
    });
  });

  test('keeps stylesheet links with compiled events native on warm mounts', async ({ page }) => {
    await openFixture(page);

    const result = await page.evaluate(async () => {
      const create = async (id: string) => {
        const child = document.createElement('test-link-event-child') as
          HTMLElement & { styleLoads?: number };
        child.id = id;
        document.body.appendChild(child);
        while ((child.styleLoads ?? 0) === 0) {
          await new Promise<void>(resolve => setTimeout(resolve));
        }
        return child;
      };
      const first = await create('first-event-child');
      const second = await create('second-event-child');
      return [first, second].map(child => ({
        adopted: child.shadowRoot?.adoptedStyleSheets.length ?? 0,
        links: child.shadowRoot?.querySelectorAll('link[rel~="stylesheet"]').length ?? 0,
        styleLoads: child.styleLoads ?? 0,
      }));
    });

    expect(result).toEqual([
      { adopted: 0, links: 1, styleLoads: 1 },
      { adopted: 0, links: 1, styleLoads: 1 },
    ]);
  });
});

interface FrameObserverResult {
  adopted: number;
  exposed: boolean;
  links: number;
  timedOut: boolean;
}

interface FrameObserverWindow {
  __webuiCssLinkResult?: FrameObserverResult;
}

async function installFrameObserver(page: Page): Promise<void> {
  await page.locator('test-link-host').evaluate((host) => {
    const target = window as typeof window & FrameObserverWindow;
    let frames = 0;
    const sample = (): void => {
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      const label = child?.shadowRoot?.querySelector('.child-label');
      if (child && label instanceof HTMLElement) {
        const visible = getComputedStyle(child).visibility !== 'hidden';
        const styled = getComputedStyle(label).color === 'rgb(128, 0, 128)';
        const previous = target.__webuiCssLinkResult;
        const exposed = previous?.exposed === true || (visible && !styled);
        if (visible && styled) {
          target.__webuiCssLinkResult = {
            adopted: child.shadowRoot?.adoptedStyleSheets.length ?? 0,
            exposed,
            links: child.shadowRoot?.querySelectorAll('link[rel~="stylesheet"]').length ?? 0,
            timedOut: false,
          };
          return;
        }
        target.__webuiCssLinkResult = {
          adopted: 0,
          exposed,
          links: 0,
          timedOut: false,
        };
      }
      frames += 1;
      if (frames >= 240) {
        target.__webuiCssLinkResult = {
          adopted: 0,
          exposed: target.__webuiCssLinkResult?.exposed === true,
          links: 0,
          timedOut: true,
        };
        return;
      }
      requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  });
}

async function readFrameObserver(page: Page): Promise<FrameObserverResult> {
  await page.waitForFunction(() => {
    const result = (window as typeof window & FrameObserverWindow).__webuiCssLinkResult;
    return result?.timedOut === true ||
      (result !== undefined && result.adopted + result.links > 0);
  });
  return page.evaluate(() => {
    return (window as typeof window & FrameObserverWindow).__webuiCssLinkResult!;
  });
}
