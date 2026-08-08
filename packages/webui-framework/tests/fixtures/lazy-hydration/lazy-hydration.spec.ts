// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';

const FIXTURE = '/lazy-hydration/fixture.html';
const ORDINARY_DATA_WS_FIXTURE =
  '/lazy-hydration-ordinary-data-ws/fixture.html';

async function installControllableObserver(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const observed = window as unknown as {
      __triggerLazyIntersections?: (targets: Element[]) => void;
      __settleLazyIntersections?: () => void;
      __isLazyObserved?: (target: Element) => boolean;
    };
    const targets = new Set<Element>();
    let notify: IntersectionObserverCallback | undefined;
    class ControllableObserver {
      readonly root = null;
      readonly rootMargin = '200px';
      readonly thresholds = [0];

      constructor(callback: IntersectionObserverCallback) {
        notify = callback;
        observed.__triggerLazyIntersections = (targets: Element[]): void => {
          const entries = targets.map((target) => ({
            target,
            isIntersecting: true,
            intersectionRatio: 1,
          })) as IntersectionObserverEntry[];
          callback(entries, this as unknown as IntersectionObserver);
        };
        observed.__settleLazyIntersections = (): void => {
          const entries = Array.from(targets, (target) => ({
            target,
            isIntersecting: false,
            intersectionRatio: 0,
          })) as IntersectionObserverEntry[];
          notify?.(entries, this as unknown as IntersectionObserver);
        };
        observed.__isLazyObserved = (target: Element): boolean =>
          targets.has(target);
      }

      observe(target: Element): void {
        targets.add(target);
      }
      unobserve(target: Element): void {
        targets.delete(target);
      }
      disconnect(): void {
        targets.clear();
      }
      takeRecords(): IntersectionObserverEntry[] {
        return [];
      }
    }
    Object.defineProperty(window, 'IntersectionObserver', {
      configurable: true,
      value: ControllableObserver,
    });
  });
}

test.describe('component lazy hydration', () => {
  test('completes the startup cohort without waiting for offscreen roots', async ({ page }) => {
    await page.addInitScript(() => {
      const observed = window as unknown as {
        __hydrationCompleteCount?: number;
        __hydratedAtComplete?: string[];
      };
      window.addEventListener('webui:hydration-complete', () => {
        observed.__hydrationCompleteCount =
          (observed.__hydrationCompleteCount ?? 0) + 1;
        observed.__hydratedAtComplete = Array.from(
          document.querySelectorAll('[data-hydrated]'),
          (element) => element.id,
        );
      });
    });
    await page.goto(FIXTURE);

    await expect.poll(() =>
      page.evaluate(() =>
        (window as unknown as { __hydrationCompleteCount?: number })
          .__hydrationCompleteCount ?? 0
      )
    ).toBe(1);
    const hydrated = await page.evaluate(() =>
      (window as unknown as { __hydratedAtComplete?: string[] })
        .__hydratedAtComplete
    );
    expect(hydrated).toContain('visible');
    expect(hydrated).toContain('near');
    expect(hydrated).not.toContain('offscreen');
  });

  test('completes startup when every lazy root is offscreen', async ({ page }) => {
    await installControllableObserver(page);
    await page.addInitScript(() => {
      const observed = window as unknown as {
        __allOffscreenCompleteCount?: number;
      };
      window.addEventListener('webui:hydration-complete', () => {
        observed.__allOffscreenCompleteCount =
          (observed.__allOffscreenCompleteCount ?? 0) + 1;
      });
    });
    await page.goto(FIXTURE);
    await page.evaluate(() => {
      (
        window as unknown as {
          __settleLazyIntersections?: () => void;
        }
      ).__settleLazyIntersections?.();
    });

    await expect.poll(() =>
      page.evaluate(() =>
        (
          window as unknown as {
            __allOffscreenCompleteCount?: number;
          }
        ).__allOffscreenCompleteCount ?? 0
      )
    ).toBe(1);
    // Hydration-only roots remain controlled by the mocked observer. Complete
    // rendering-policy roots may also follow the browser's independent native
    // content-visibility relevance signal.
    const unexpectedlyHydrated = await page.evaluate((ids) =>
      ids.filter((id) =>
        document.getElementById(id)?.hasAttribute('data-hydrated'),
      ), [
        'visible',
        'near',
        'offscreen',
        'scrolled',
        'nested',
        'reconnect',
        'list-root',
        'early-image',
        'early-image-error',
        'streamed',
        'barrier-parent',
        'streamed-nested',
      ]);
    expect(unexpectedlyHydrated).toEqual([]);
  });

  test('keeps SSR visible and hydrates only the viewport plus 200px margin', async ({ page }) => {
    await page.goto(FIXTURE);

    await expect(page.locator('#offscreen .label')).toHaveText('Offscreen');
    await expect(page.locator('#visible')).toHaveAttribute('data-hydrated', '');
    await expect(page.locator('#near')).toHaveAttribute('data-hydrated', '');
    await expect(page.locator('#offscreen')).not.toHaveAttribute('data-hydrated', '');

    await page.evaluate(() => window.scrollTo(0, 1_100));
    await expect(page.locator('#offscreen')).toHaveAttribute('data-hydrated', '');
  });

  test('uses one observer with viewport and nested-scroll lead margins', async ({ page }) => {
    await page.addInitScript(() => {
      const NativeObserver = window.IntersectionObserver;
      const observed = window as unknown as {
        __observerCount?: number;
        __observerOptions?: IntersectionObserverInit;
      };
      window.IntersectionObserver = new Proxy(NativeObserver, {
        construct(target, args) {
          observed.__observerCount = (observed.__observerCount ?? 0) + 1;
          observed.__observerOptions = args[1] as IntersectionObserverInit;
          return Reflect.construct(target, args) as IntersectionObserver;
        },
      });
    });
    await page.goto(FIXTURE);

    const observer = await page.evaluate(() => {
      const observed = window as unknown as {
        __observerCount?: number;
        __observerOptions?: IntersectionObserverInit & { scrollMargin?: string };
      };
      return {
        count: observed.__observerCount,
        rootMargin: observed.__observerOptions?.rootMargin,
        scrollMargin: observed.__observerOptions?.scrollMargin ?? null,
      };
    });
    expect(observer.count).toBe(1);
    if (observer.scrollMargin === null) {
      expect(observer.rootMargin).toBe('200px');
    } else {
      expect(observer).toMatchObject({
        rootMargin: '0px',
        scrollMargin: '200px',
      });
    }

    await expect(page.locator('#scrolled')).not.toHaveAttribute('data-hydrated', '');
    await page.locator('#nested-scroller').evaluate((element) => {
      element.scrollTop = 400;
    });
    await expect(page.locator('#scrolled')).toHaveAttribute('data-hydrated', '');
  });

  test('hydrates synchronously before an offscreen interaction reaches its target', async ({ page }) => {
    await page.goto(FIXTURE);
    await expect(page.locator('#offscreen')).not.toHaveAttribute('data-hydrated', '');

    const hydratedBeforeClick = await page.locator('#offscreen button').evaluate((button) => {
      const host = document.getElementById('offscreen');
      if (!host) throw new Error('offscreen host missing');
      button.dispatchEvent(new PointerEvent('pointerdown', {
        bubbles: true,
        composed: true,
      }));
      return host.hasAttribute('data-hydrated');
    });
    expect(hydratedBeforeClick).toBe(true);
    await page.locator('#offscreen button').evaluate((button) => {
      (button as HTMLElement).click();
    });
    await expect(page.locator('#offscreen .count')).toHaveText('1');
  });

  test('installs the first direct focus handler before target dispatch', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);
    await expect(page.locator('#offscreen')).not.toHaveAttribute('data-hydrated', '');

    const hydratedDuringFocus = await page.locator('#offscreen .focus-target').evaluate((input) => {
      const host = document.getElementById('offscreen');
      (input as HTMLElement).focus();
      return host?.hasAttribute('data-hydrated') ?? false;
    });

    expect(hydratedDuringFocus).toBe(true);
    await expect(page.locator('#offscreen .focus-count')).toHaveText('1');
  });

  test('installs the first mouseenter handler during captured pointerover', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);
    const host = page.locator('#offscreen');
    await expect(host).not.toHaveAttribute('data-hydrated', '');

    await host.locator('.hover-target').hover();

    await expect(host).toHaveAttribute('data-hydrated', '');
    await expect(host.locator('.hover-count')).toHaveText('1');
  });

  test('reconciles images that complete or fail before deferred hydration', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);
    const host = page.locator('#early-image');
    const image = host.locator('.image');
    const errorHost = page.locator('#early-image-error');
    const errorImage = errorHost.locator('.image');
    await expect(host).not.toHaveAttribute('data-hydrated', '');
    await expect(errorHost).not.toHaveAttribute('data-hydrated', '');
    await expect(host.locator('.image-status')).toHaveText('pending');
    await errorImage.evaluate((element) => {
      (element as HTMLImageElement).src =
        '/lazy-hydration/missing-before-hydration.gif';
    });

    await expect.poll(() =>
      image.evaluate((element) => (element as HTMLImageElement).complete)
    ).toBe(true);
    expect(
      await image.evaluate(
        (element) => (element as HTMLImageElement).naturalWidth,
      ),
    ).toBeGreaterThan(0);
    await expect.poll(() =>
      errorImage.evaluate((element) => {
        const target = element as HTMLImageElement;
        return target.complete && target.naturalWidth === 0;
      })
    ).toBe(true);
    await expect(host.locator('.image-status')).toHaveText('pending');
    await expect(errorHost.locator('.image-status')).toHaveText('pending');

    await page.evaluate(() => {
      const target = document.getElementById('early-image');
      const errorTarget = document.getElementById('early-image-error');
      const trigger = (
        window as unknown as {
          __triggerLazyIntersections?: (targets: Element[]) => void;
        }
      ).__triggerLazyIntersections;
      if (!target || !errorTarget || !trigger) {
        throw new Error('lazy image target is missing');
      }
      trigger([target, errorTarget]);
    });

    await expect(host).toHaveAttribute('data-hydrated', '');
    await expect(errorHost).toHaveAttribute('data-hydrated', '');
    await expect(host.locator('.image-status')).toHaveText('loaded');
    await expect(errorHost.locator('.image-status')).toHaveText('error');

    await image.evaluate((element) => {
      element.dispatchEvent(new Event('error'));
    });
    await expect(host.locator('.image-status')).toHaveText('error');
    await image.evaluate((element) => {
      element.dispatchEvent(new Event('load'));
    });
    await expect(host.locator('.image-status')).toHaveText('loaded');
  });

  test('activates nested lazy components parent-first', async ({ page }) => {
    await page.goto(FIXTURE);

    await expect(page.locator('#nested')).not.toHaveAttribute(
      'data-hydrated',
      '',
    );
    await expect(page.locator('#nested-child')).not.toHaveAttribute(
      'data-hydrated',
      '',
    );
    await page.evaluate(() => {
      if (window.__webui) delete window.__webui.state;
    });
    await page.locator('#nested-child button').evaluate((button) => {
      (button as HTMLElement).click();
    });
    const log = await page.evaluate(() => window.__lazyHydrationLog);
    const parent = log?.indexOf('nested') ?? -1;
    const child = log?.indexOf('nested-child') ?? -1;
    expect(parent).toBeGreaterThanOrEqual(0);
    expect(child).toBeGreaterThan(parent);
    await expect(page.locator('#nested')).toHaveAttribute('data-hydrated', '');
    await expect(page.locator('#nested-child')).toHaveAttribute('data-hydrated', '');
    expect(
      await page.locator('#nested-child').evaluate(
        (element) => (element as HTMLElement & { note: string }).note,
      ),
    ).toBe('SSR note');
  });

  test('keeps streamed eager children behind a visible parent barrier', async ({ page }) => {
    await page.goto(FIXTURE);

    const boundary = await page.evaluate(() => {
      const activate = Symbol.for('microsoft.webui.boundaryActivate');
      const streamingMode = window.__enableStreamingModeForTest?.();
      type BoundaryActivatable = HTMLElement & {
        [key: symbol]: (state?: Record<string, unknown>) => number;
      };
      const parent = document.querySelector('#streamed-nested') as
        BoundaryActivatable;
      const child = parent.shadowRoot?.querySelector(
        '#streamed-nested-child',
      ) as BoundaryActivatable;
      const boundaryState = { note: 'Streamed boundary note' };
      parent.setAttribute('data-ws', 'test-boundary');
      child.setAttribute('data-ws', 'test-boundary');
      const hydratedBefore = [
        parent.hasAttribute('data-hydrated'),
        child.hasAttribute('data-hydrated'),
      ];
      const deferredBefore = [
        (parent as unknown as { $deferredSSR: boolean }).$deferredSSR,
        (child as unknown as { $deferredSSR: boolean }).$deferredSSR,
      ];
      const hadMarkers = [
        parent.hasAttribute('data-ws'),
        child.hasAttribute('data-ws'),
      ];
      const parentOutcome = parent[activate](boundaryState);
      const hydratedAfterParent = [
        parent.hasAttribute('data-hydrated'),
        child.hasAttribute('data-hydrated'),
      ];
      const outcomes = [
        parentOutcome,
        child[activate](boundaryState),
      ];
      parent.removeAttribute('data-ws');
      child.removeAttribute('data-ws');
      if (window.__webui) delete window.__webui.state;
      return {
        deferredBefore,
        hadMarkers,
        hydratedBefore,
        hydratedAfterParent,
        hydratedAfter: [
          parent.hasAttribute('data-hydrated'),
          child.hasAttribute('data-hydrated'),
        ],
        outcomes,
        streamingMode,
      };
    });

    expect(boundary).toEqual({
      deferredBefore: [true, true],
      hadMarkers: [true, true],
      hydratedBefore: [false, false],
      hydratedAfterParent: [false, false],
      hydratedAfter: [false, false],
      outcomes: [1, 1],
      streamingMode: true,
    });
    await expect(page.locator('#streamed-nested')).not.toHaveAttribute(
      'data-hydrated',
      '',
    );
    await expect(page.locator('#streamed-nested-child')).not.toHaveAttribute(
      'data-hydrated',
      '',
    );
    await page.locator('#streamed-nested-child button').evaluate((button) => {
      (button as HTMLElement).click();
    });

    const log = await page.evaluate(() => window.__lazyHydrationLog);
    const parent = log?.indexOf('streamed-nested') ?? -1;
    const child = log?.indexOf('streamed-nested-child') ?? -1;
    expect(parent).toBeGreaterThanOrEqual(0);
    expect(child).toBeGreaterThan(parent);
    await expect(page.locator('#streamed-nested-child .count')).toHaveText('1');
    expect(
      await page.locator('#streamed-nested-child').evaluate(
        (element) => (element as HTMLElement & { note: string }).note,
      ),
    ).toBe('Streamed boundary note');
  });

  test('releases every eager child when parent and sibling callbacks fail', async ({ page }) => {
    await page.goto(FIXTURE);

    await expect(page.locator('#barrier-parent')).not.toHaveAttribute(
      'data-hydrated',
      '',
    );
    await expect(page.locator('#barrier-healthy-child')).not.toHaveAttribute(
      'data-hydrated',
      '',
    );
    await page.locator('#barrier-healthy-child button').evaluate((button) => {
      (button as HTMLElement).click();
    });

    await expect(page.locator('#barrier-failing-child')).toHaveAttribute(
      'data-hydrated',
      '',
    );
    await expect(page.locator('#barrier-healthy-child')).toHaveAttribute(
      'data-hydrated',
      '',
    );
    await expect(page.locator('#barrier-healthy-child .count')).toHaveText('1');
  });

  test('applies the build-time offscreen policy and honors both eager escape hatches', async ({ page }) => {
    await page.goto(FIXTURE);

    await expect(page.locator('style[data-webui-render-policy]')).toHaveCount(1);
    await expect(page.locator('#render-offscreen button')).toHaveAccessibleName(
      'Rendered offscreen',
    );
    await expect(
      page.locator('#render-offscreen').getByRole('status'),
    ).toHaveText('0');
    await expect(page.locator('#render-offscreen')).not.toHaveAttribute(
      'data-hydrated',
      '',
    );
    await expect(page.locator('#render-hydrate-eager')).toHaveAttribute(
      'data-hydrated',
      '',
    );
    await expect(page.locator('#render-eager')).toHaveAttribute(
      'data-hydrated',
      '',
    );

    const styles = await page.evaluate(() => {
      const offscreen = getComputedStyle(
        document.querySelector('#render-offscreen')!,
      );
      const hydrateEager = getComputedStyle(
        document.querySelector('#render-hydrate-eager')!,
      );
      const renderEager = getComputedStyle(
        document.querySelector('#render-eager')!,
      );
      const shadowParent = document.querySelector(
        '#shadow-policy-parent',
      )?.shadowRoot;
      const shadowOffscreen = getComputedStyle(
        shadowParent?.querySelector('#shadow-render-offscreen')!,
      );
      const shadowEager = getComputedStyle(
        shadowParent?.querySelector('#shadow-render-eager')!,
      );
      return {
        offscreen: [
          offscreen.contentVisibility,
          offscreen.containIntrinsicBlockSize,
        ],
        hydrateEager: hydrateEager.contentVisibility,
        renderEager: renderEager.contentVisibility,
        shadowOffscreen: [
          shadowOffscreen.contentVisibility,
          shadowOffscreen.containIntrinsicBlockSize,
        ],
        shadowEager: shadowEager.contentVisibility,
      };
    });
    expect(styles.offscreen).toEqual(['auto', 'auto 72px']);
    expect(styles.hydrateEager).toBe('auto');
    expect(styles.renderEager).toBe('visible');
    expect(styles.shadowOffscreen).toEqual(['auto', 'auto 72px']);
    expect(styles.shadowEager).toBe('visible');
  });

  test('uses native content-visibility relevance after initial observer classification', async ({ page }) => {
    await page.goto(FIXTURE);
    await expect(page.locator('#render-offscreen')).not.toHaveAttribute(
      'data-hydrated',
      '',
    );
    await expect.poll(() =>
      page.evaluate(() =>
        'ContentVisibilityAutoStateChangeEvent' in window
      )
    ).toBe(true);

    await page.evaluate(() => window.scrollTo(0, 6_350));
    await expect(page.locator('#render-offscreen')).toHaveAttribute(
      'data-hydrated',
      '',
    );
    await page.locator('#render-offscreen button').click();
    await expect(page.locator('#render-offscreen .count')).toHaveText('1');
  });

  test('ignores bubbled content-visibility events from light DOM descendants', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);

    const result = await page.evaluate(() => {
      const activateHost = document.querySelector('#offscreen');
      const retainedHost = document.querySelector('#reconnect');
      const isObserved = (
        window as unknown as {
          __isLazyObserved?: (target: Element) => boolean;
        }
      ).__isLazyObserved;
      if (!activateHost || !retainedHost || !isObserved) {
        throw new Error('lazy hydration probes are missing');
      }
      const dispatch = (host: Element, skipped: boolean): void => {
        const child = document.createElement('span');
        host.appendChild(child);
        const event = new Event('contentvisibilityautostatechange', {
          bubbles: true,
        });
        Object.defineProperty(event, 'skipped', { value: skipped });
        child.dispatchEvent(event);
      };
      dispatch(activateHost, false);
      dispatch(retainedHost, true);
      return {
        activated: activateHost.hasAttribute('data-hydrated'),
        fallbackRetained: isObserved(retainedHost),
      };
    });

    expect(result).toEqual({
      activated: false,
      fallbackRetained: true,
    });
  });

  test('retains the observer fallback when native relevance predates definition', async ({ page }) => {
    await page.goto(FIXTURE);
    const host = page.locator('#render-native-near');
    const button = host.locator('button');
    expect(
      await button.evaluate((element) =>
        element.checkVisibility({ contentVisibilityAuto: true })
      ),
    ).toBe(true);

    await page.evaluate(() => window.__defineLateOffscreenItem?.());
    await expect(host).not.toHaveAttribute('data-hydrated', '');
    await page.evaluate(() => new Promise<void>((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
    }));

    await page.evaluate(() => window.scrollTo(0, 400));
    await expect(host).toHaveAttribute('data-hydrated', '');
  });

  test('activates observer targets through shadow roots parent-first', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);

    const order = await page.evaluate(() => {
      window.__createShadowLazyProbe?.();
      const child = window.__shadowProbeChild;
      if (!child) throw new Error('shadow probe was not created');
      const trigger = (
        window as unknown as {
          __triggerLazyIntersections?: (targets: Element[]) => void;
        }
      ).__triggerLazyIntersections;
      if (!trigger) throw new Error('controllable observer is missing');
      trigger([child]);
      return window.__lazyHydrationLog?.slice(-2);
    });

    expect(order).toEqual(['shadow-probe-parent', 'shadow-probe-child']);
  });

  test('re-observes a deferred component after reconnect', async ({ page }) => {
    await page.goto(FIXTURE);
    await page.locator('#reconnect').evaluate((element) => {
      const parent = element.parentNode;
      element.remove();
      (element as HTMLElement & { note: string }).note = 'Detached update';
      parent?.appendChild(element);
    });
    await expect(page.locator('#reconnect')).not.toHaveAttribute('data-hydrated', '');

    await page.locator('#reconnect button').evaluate((button) => {
      (button as HTMLElement).click();
    });
    await expect(page.locator('#reconnect')).toHaveAttribute('data-hydrated', '');
    await expect(page.locator('#reconnect .count')).toHaveText('1');
    await expect(page.locator('#reconnect .note')).toHaveText('Detached update');
  });

  test('preserves live state and hydrates immediately after mounted teardown', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);
    await page.evaluate(() => {
      const target = document.getElementById('reconnect');
      const trigger = (
        window as unknown as {
          __triggerLazyIntersections?: (targets: Element[]) => void;
        }
      ).__triggerLazyIntersections;
      if (!target || !trigger) throw new Error('reconnect lazy target is missing');
      trigger([target]);
    });
    await expect(page.locator('#reconnect')).toHaveAttribute('data-hydrated', '');

    const state = await page.locator('#reconnect').evaluate(async (element) => {
      const host = element as HTMLElement & {
        note: string;
        $deferredSSR: boolean;
        $flushUpdates(): void;
        $hydrated: boolean;
      };
      host.note = 'Live reconnect state';
      host.$flushUpdates();
      const bootstrap = window as unknown as {
        __webui?: { state?: Record<string, unknown> };
      };
      const webui = bootstrap.__webui ??= {};
      (webui.state ??= {}).note = 'Stale bootstrap state';
      const parent = host.parentNode;
      const next = host.nextSibling;
      if (!parent) throw new Error('reconnect lazy parent is missing');
      host.remove();
      await Promise.resolve();
      const tornDown = !host.$hydrated;
      parent.insertBefore(host, next);
      return {
        deferred: host.$deferredSSR,
        hydrated: host.$hydrated,
        note: host.note,
        text: host.shadowRoot?.querySelector('.note')?.textContent,
        tornDown,
      };
    });

    expect(state).toEqual({
      deferred: false,
      hydrated: true,
      note: 'Live reconnect state',
      text: 'Live reconnect state',
      tornDown: true,
    });
    await page.locator('#reconnect button').evaluate((button) => {
      (button as HTMLElement).click();
    });

    await expect(page.locator('#reconnect .count')).toHaveText('1');
  });

  test('remounts repeat and conditional structures after delayed reconnect', async ({ page }) => {
    await page.goto(FIXTURE);
    const list = page.locator('#list-root');
    await list.locator('.replace').evaluate((button) => {
      button.dispatchEvent(new PointerEvent('pointerdown', {
        bubbles: true,
        composed: true,
      }));
    });
    await expect(list).toHaveAttribute('data-hydrated', '');

    await list.evaluate(async (element) => {
      const host = element as HTMLElement & {
        items: Array<{ id: number; label: string; count: number }>;
        showSummary: boolean;
      };
      host.items = [
        { id: 20, label: 'Twenty', count: 20 },
        { id: 21, label: 'Twenty one', count: 21 },
      ];
      host.showSummary = false;
      const parent = host.parentNode;
      host.remove();
      await Promise.resolve();
      parent?.appendChild(host);
    });

    await list.locator('.replace').evaluate((button) => {
      button.dispatchEvent(new PointerEvent('pointerdown', {
        bubbles: true,
        composed: true,
      }));
    });
    await expect(list.locator('test-lazy-item')).toHaveCount(2);
    expect(
      await list.locator('test-lazy-item').evaluateAll((elements) =>
        elements.map((element) => element.getAttribute('label'))
      ),
    ).toEqual(
      ['Twenty', 'Twenty one'],
    );
    await expect(list.locator('.summary')).toHaveCount(0);

    await list.locator('.toggle-summary').click();
    await expect(list.locator('.summary')).toHaveText('12 items');
  });

  test('replays parent repeat updates into an offscreen deferred child', async ({ page }) => {
    await page.goto(FIXTURE);
    const last = page.locator('#list-root test-lazy-item').last();
    await expect(page.locator('#list-root')).not.toHaveAttribute('data-hydrated', '');
    await expect(last).not.toHaveAttribute('data-hydrated', '');

    await page.locator('#list-root .replace').evaluate((button) => {
      button.dispatchEvent(new PointerEvent('pointerdown', {
        bubbles: true,
        composed: true,
      }));
    });
    await expect(page.locator('#list-root')).toHaveAttribute('data-hydrated', '');
    await page.locator('#list-root .replace').evaluate((button) => {
      (button as HTMLElement).click();
    });
    await last.locator('button').evaluate((button) => {
      (button as HTMLElement).click();
    });

    await expect(last).toHaveAttribute('data-hydrated', '');
    await expect(last.locator('.label')).toHaveText('Updated 11');
    await expect(last.locator('.count')).toHaveText('22');
  });

  test('clears scoped text and attributes for explicit and sparse undefined items', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);
    const list = page.locator('#list-root');
    await expect(list).not.toHaveAttribute('data-hydrated', '');
    await expect(list.locator('.explicit-undefined-item')).toHaveText(
      'Explicit SSR value',
    );
    await expect(list.locator('.sparse-item')).toHaveText('Sparse SSR value');

    await list.evaluate((element) => {
      const host = element as HTMLElement & {
        explicitUndefinedItems: Array<string | undefined>;
        sparseItems: Array<string | undefined>;
      };
      host.explicitUndefinedItems = [undefined];
      host.sparseItems = new Array<string | undefined>(1);
      host.dispatchEvent(new PointerEvent('pointerdown', {
        bubbles: true,
        composed: true,
      }));
    });

    await expect(list).toHaveAttribute('data-hydrated', '');
    await expect(list.locator('.explicit-undefined-item')).toHaveText('');
    await expect(list.locator('.explicit-undefined-item')).toHaveAttribute(
      'data-value',
      '',
    );
    await expect(list.locator('.sparse-item')).toHaveText('');
    await expect(list.locator('.sparse-item')).toHaveAttribute('data-value', '');
  });

  test('retains streamed state and lets newer patches win before visibility', async ({ page }) => {
    await page.goto(FIXTURE);
    await expect(page.locator('#streamed')).not.toHaveAttribute('data-hydrated', '');

    const outcome = await page.locator('#streamed').evaluate((element) => {
      const streamingMode = window.__enableStreamingModeForTest?.();
      const host = element as HTMLElement & {
        note: string;
        sameValue: string;
        setState(state: Record<string, unknown>): void;
        [key: symbol]: ((state?: Record<string, unknown>) => number) | unknown;
      };
      const activate = host[Symbol.for('microsoft.webui.boundaryActivate')];
      if (typeof activate !== 'function') throw new Error('missing boundary activation hook');
      host.setAttribute('data-ws', 'test-boundary');
      const hadMarker = host.hasAttribute('data-ws');
      const result = activate.call(host, {
        label: 'Boundary',
        count: 2,
        note: 'Older boundary note',
        sameValue: 'Older boundary value',
      });
      const hydratedAtBoundary = host.hasAttribute('data-hydrated');
      host.removeAttribute('data-ws');
      host.note = '';
      host.sameValue = '';
      host.setState({ label: 'Patched', count: 9 });
      const hydratedAfterState = host.hasAttribute('data-hydrated');
      host.dispatchEvent(new PointerEvent('pointerdown', {
        bubbles: true,
        composed: true,
      }));
      return {
        result,
        streamingMode,
        hadMarker,
        hydratedAtBoundary,
        hydratedAfterState,
      };
    });

    expect(outcome).toEqual({
      result: 1,
      streamingMode: true,
      hadMarker: true,
      hydratedAtBoundary: false,
      hydratedAfterState: false,
    });
    await expect(page.locator('#streamed')).toHaveAttribute('data-hydrated', '');
    await expect(page.locator('#streamed .mixed')).toHaveText(
      'Streamed SSR / Server only',
    );
    await page.locator('#streamed button').evaluate((button) => {
      (button as HTMLElement).click();
    });
    await expect(page.locator('#streamed .label')).toHaveText('Patched');
    await expect(page.locator('#streamed .count')).toHaveText('10');
    await expect(page.locator('#streamed .server-only')).toHaveText('Server only');
    await expect(page.locator('#streamed .note')).toHaveText('');
    await expect(page.locator('#streamed .same-value')).toHaveText('');
    await expect(page.locator('#streamed .mixed')).toHaveText(
      'Streamed SSR / Server only',
    );
    await page.locator('#streamed').evaluate((element) => {
      const host = element as HTMLElement & {
        label: string;
        $flushUpdates(): void;
      };
      host.label = 'Second patch';
      host.$flushUpdates();
    });
    await expect(page.locator('#streamed .label')).toHaveText('Second patch');
    await expect(page.locator('#streamed .mixed')).toHaveText(
      'Streamed SSR / Server only',
    );
  });

  test('isolates one failed activation from other visible targets', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);

    const result = await page.evaluate(() => {
      let reported = '';
      Object.defineProperty(window, 'reportError', {
        configurable: true,
        value(error: unknown) {
          reported = error instanceof Error ? error.message : String(error);
        },
      });
      window.__createFailureLazyProbes?.();
      const probes = window.__failureLazyProbes;
      const trigger = (
        window as unknown as {
          __triggerLazyIntersections?: (targets: Element[]) => void;
        }
      ).__triggerLazyIntersections;
      if (!probes || !trigger) throw new Error('failure probes are missing');
      trigger([probes[1]]);
      return {
        healthy: window.__healthyLazyActivated ?? 0,
        reported,
      };
    });

    expect(result).toEqual({
      healthy: 1,
      reported: 'intentional lazy activation failure',
    });
  });

  test('yields when a visible activation batch exceeds the main-thread budget', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);

    const immediate = await page.evaluate(() => {
      window.__createSlowLazyProbes?.(40);
      const probes = window.__slowLazyProbes;
      const trigger = (
        window as unknown as {
          __triggerLazyIntersections?: (targets: Element[]) => void;
        }
      ).__triggerLazyIntersections;
      if (!probes || !trigger) throw new Error('slow probes are missing');
      trigger(probes);
      return window.__slowLazyActivated ?? 0;
    });

    expect(immediate).toBeGreaterThan(0);
    expect(immediate).toBeLessThan(40);
    await expect.poll(() =>
      page.evaluate(() => window.__slowLazyActivated ?? 0)
    ).toBe(40);
  });

  test('time-slices a deeply nested parent-first activation chain', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);

    const immediate = await page.evaluate(() => {
      window.__createNestedSlowLazyProbes?.(40);
      const deepest = window.__nestedSlowDeepest;
      const trigger = (
        window as unknown as {
          __triggerLazyIntersections?: (targets: Element[]) => void;
        }
      ).__triggerLazyIntersections;
      if (!deepest || !trigger) throw new Error('nested slow probes are missing');
      trigger([deepest]);
      return window.__slowLazyActivated ?? 0;
    });

    expect(immediate).toBeGreaterThan(0);
    expect(immediate).toBeLessThan(40);
    await expect.poll(() =>
      page.evaluate(() => window.__slowLazyActivated ?? 0)
    ).toBe(40);
    const order = await page.evaluate(() =>
      window.__lazyHydrationLog?.filter((id) =>
        id.startsWith('nested-slow-probe-')
      )
    );
    expect(order).toEqual(
      Array.from({ length: 40 }, (_, index) => `nested-slow-probe-${index}`),
    );
  });

  test('invalidates queued visibility work across reconnect', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);

    const immediate = await page.evaluate(() => {
      window.__createSlowLazyProbes?.(40);
      const probes = window.__slowLazyProbes;
      const trigger = (
        window as unknown as {
          __triggerLazyIntersections?: (targets: Element[]) => void;
        }
      ).__triggerLazyIntersections;
      if (!probes || !trigger) throw new Error('slow probes are missing');
      trigger(probes);
      window.__reconnectSlowLazyProbe?.(39);
      return window.__slowLazyActivated ?? 0;
    });

    expect(immediate).toBeLessThan(40);
    await expect.poll(() =>
      page.evaluate(() => window.__slowLazyActivated ?? 0)
    ).toBe(39);
    await page.evaluate(() => {
      const last = window.__slowLazyProbes?.[39];
      const trigger = (
        window as unknown as {
          __triggerLazyIntersections?: (targets: Element[]) => void;
        }
      ).__triggerLazyIntersections;
      if (!last || !trigger) throw new Error('reconnected probe is missing');
      trigger([last]);
    });
    await expect.poll(() =>
      page.evaluate(() => window.__slowLazyActivated ?? 0)
    ).toBe(40);
  });

  test('keeps an observer batch open across interaction activation', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);

    const state = await page.evaluate(() => {
      window.__resetLazyHydrationLifecycle?.();
      window.__createSlowLazyProbes?.(40);
      const probes = window.__slowLazyProbes;
      const trigger = (
        window as unknown as {
          __triggerLazyIntersections?: (targets: Element[]) => void;
        }
      ).__triggerLazyIntersections;
      if (!probes || !trigger) throw new Error('slow probes are missing');
      trigger(probes);
      const beforeInteraction = window.__lazyHydrationPendingCount?.();
      document.getElementById('offscreen')?.dispatchEvent(
        new PointerEvent('pointerdown', {
          bubbles: true,
          composed: true,
        }),
      );
      return {
        beforeInteraction,
        afterInteraction: window.__lazyHydrationPendingCount?.(),
      };
    });

    expect(state).toEqual({
      beforeInteraction: 1,
      afterInteraction: 1,
    });
    await expect.poll(() =>
      page.evaluate(() => window.__lazyHydrationPendingCount?.())
    ).toBe(0);
  });

  test('recovers when a scheduled continuation is rejected', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);

    const immediate = await page.evaluate(() => {
      const target = globalThis as typeof globalThis & {
        scheduler?: unknown;
      };
      const observed = window as unknown as {
        __continuationError?: string;
        __restoreScheduler?: () => void;
        __triggerLazyIntersections?: (targets: Element[]) => void;
      };
      const originalScheduler = target.scheduler;
      Object.defineProperty(target, 'scheduler', {
        configurable: true,
        value: {
          postTask: () =>
            Promise.reject(new Error('intentional scheduler rejection')),
        },
      });
      observed.__restoreScheduler = (): void => {
        Object.defineProperty(target, 'scheduler', {
          configurable: true,
          value: originalScheduler,
        });
      };
      Object.defineProperty(window, 'reportError', {
        configurable: true,
        value(error: unknown) {
          observed.__continuationError =
            error instanceof Error ? error.message : String(error);
        },
      });

      window.__createSlowLazyProbes?.(40);
      const probes = window.__slowLazyProbes;
      if (!probes || !observed.__triggerLazyIntersections) {
        throw new Error('slow probes are missing');
      }
      observed.__triggerLazyIntersections(probes);
      return window.__slowLazyActivated ?? 0;
    });

    expect(immediate).toBeGreaterThan(0);
    expect(immediate).toBeLessThan(40);
    await expect.poll(() =>
      page.evaluate(() =>
        (
          window as unknown as {
            __continuationError?: string;
          }
        ).__continuationError
      )
    ).toBe('intentional scheduler rejection');

    await expect.poll(() =>
      page.evaluate(() => window.__slowLazyActivated ?? 0)
    ).toBe(40);
    await page.evaluate(() => {
      (
        window as unknown as {
          __restoreScheduler?: () => void;
        }
      ).__restoreScheduler?.();
    });
  });

  test('treats authored data-ws as ordinary markup and retains bootstrap state', async ({ page }) => {
    await page.goto(ORDINARY_DATA_WS_FIXTURE);
    const host = page.locator('#ordinary-lazy');
    await expect(host.locator('.label')).toHaveText('Retained SSR state');
    await expect(host).not.toHaveAttribute('data-hydrated', '');

    await page.evaluate(() => {
      const bootstrap = window as unknown as {
        __webui?: { state?: Record<string, unknown> };
      };
      if (bootstrap.__webui) delete bootstrap.__webui.state;
      window.scrollTo(0, 1_500);
    });

    await expect(host).toHaveAttribute('data-hydrated', '');
    await expect(host).toHaveAttribute('data-ws', 'authored-value');
    await expect(host.locator('.label')).toHaveText('Retained SSR state');
  });

  test('completes startup after visible lazy roots without waiting offscreen', async ({ page }) => {
    await installControllableObserver(page);
    await page.addInitScript(() => {
      const observed = window as unknown as {
        __ordinaryCompleteCount?: number;
        __ordinaryHydratedAtComplete?: string[];
      };
      window.addEventListener('webui:hydration-complete', () => {
        observed.__ordinaryCompleteCount =
          (observed.__ordinaryCompleteCount ?? 0) + 1;
        observed.__ordinaryHydratedAtComplete = Array.from(
          document.querySelectorAll('[data-hydrated]'),
          (element) => element.id,
        );
      });
    });
    await page.goto(ORDINARY_DATA_WS_FIXTURE);
    expect(await page.evaluate(() =>
      (
        window as unknown as {
          __ordinaryCompleteCount?: number;
        }
      ).__ordinaryCompleteCount ?? 0
    )).toBe(0);
    await page.evaluate(() => {
      const visible = document.getElementById('ordinary-visible');
      const observed = window as unknown as {
        __triggerLazyIntersections?: (targets: Element[]) => void;
        __settleLazyIntersections?: () => void;
      };
      if (!visible || !observed.__triggerLazyIntersections) {
        throw new Error('visible ordinary lazy root is missing');
      }
      observed.__triggerLazyIntersections([visible]);
      observed.__settleLazyIntersections?.();
    });

    await expect.poll(() =>
      page.evaluate(() =>
        (
          window as unknown as {
            __ordinaryCompleteCount?: number;
          }
        ).__ordinaryCompleteCount ?? 0
      )
    ).toBe(1);
    const hydrated = await page.evaluate(() =>
      (
        window as unknown as {
          __ordinaryHydratedAtComplete?: string[];
        }
      ).__ordinaryHydratedAtComplete
    );
    expect(hydrated).toContain('ordinary-eager');
    expect(hydrated).toContain('ordinary-visible');
    expect(hydrated).not.toContain('ordinary-lazy');
  });

  test('falls back to eager hydration without IntersectionObserver, without warning', async ({ page }) => {
    const warnings: string[] = [];
    page.on('console', (message) => {
      if (message.type() === 'warning' || message.type() === 'error') {
        warnings.push(message.text());
      }
    });
    await page.addInitScript(() => {
      Object.defineProperty(window, 'IntersectionObserver', {
        configurable: true,
        value: undefined,
      });
    });
    await page.goto(FIXTURE);

    await expect(page.locator('#offscreen')).toHaveAttribute('data-hydrated', '');
    await expect(page.locator('#reconnect')).toHaveAttribute('data-hydrated', '');
    // A missing IntersectionObserver is an expected, documented fallback on
    // older browsers — never a misconfiguration warning.
    expect(
      warnings.filter((warning) => warning.includes('lazy-hydration.js')),
    ).toEqual([]);
  });

  test('mounts client-created components eagerly', async ({ page }) => {
    await page.goto(FIXTURE);
    const hydratedSynchronously = await page.evaluate(() => {
      const element = document.createElement('test-lazy-item');
      element.id = 'client-created';
      element.setAttribute('label', 'Client');
      element.setAttribute('count', '4');
      document.body.appendChild(element);
      return element.hasAttribute('data-hydrated');
    });
    expect(hydratedSynchronously).toBe(true);
    await expect(page.locator('#client-created .label')).toHaveText('Client');
    await expect(page.locator('#client-created .count')).toHaveText('4');
  });

  test('reconnects client-created components eagerly after teardown', async ({ page }) => {
    await installControllableObserver(page);
    await page.goto(FIXTURE);

    const state = await page.evaluate(async () => {
      const element = document.createElement('test-lazy-item');
      const host = element as HTMLElement & {
        note: string;
        $deferredSSR: boolean;
        $flushUpdates(): void;
        $hydrated: boolean;
      };
      host.id = 'client-reconnect';
      host.setAttribute('label', 'Client reconnect');
      host.style.position = 'absolute';
      host.style.top = '6800px';
      document.body.appendChild(host);
      const mountedEagerly = host.$hydrated;
      host.note = 'Client reconnect state';
      host.$flushUpdates();
      host.remove();
      await Promise.resolve();
      const tornDown = !host.$hydrated;
      document.body.appendChild(host);
      return {
        deferred: host.$deferredSSR,
        hydrated: host.$hydrated,
        mountedEagerly,
        note: host.note,
        text: host.shadowRoot?.querySelector('.note')?.textContent,
        tornDown,
      };
    });

    expect(state).toEqual({
      deferred: false,
      hydrated: true,
      mountedEagerly: true,
      note: 'Client reconnect state',
      text: 'Client reconnect state',
      tornDown: true,
    });
  });
});

test.describe('w-hydrate="eager" SSR escape hatch', () => {
  test('hydrates through a dormant compiler-owned parent', async ({ page }) => {
    await page.goto(FIXTURE);

    await expect(page.locator('#static-child')).toHaveAttribute(
      'data-hydrated',
      '',
    );
    await page.locator('#static-child button').click();
    await expect(page.locator('#static-child .count')).toHaveText('1');
  });

  test('hydrates synchronously despite being offscreen, while a normal visible sibling still defers', async ({ page }) => {
    await page.goto(FIXTURE);

    await expect(page.locator('#offscreen-eager-escape')).toHaveAttribute('data-hydrated', '');
    await expect(page.locator('#offscreen')).not.toHaveAttribute('data-hydrated', '');
  });

  test('remains eager after a delayed reconnect', async ({ page }) => {
    await page.goto(FIXTURE);
    await expect(page.locator('#offscreen-eager-escape')).toHaveAttribute('data-hydrated', '');

    await page.locator('#offscreen-eager-escape').evaluate(async (element) => {
      const parent = element.parentNode;
      const next = element.nextSibling;
      element.remove();
      await new Promise((resolve) => setTimeout(resolve, 50));
      parent?.insertBefore(element, next);
    });

    await expect(page.locator('#offscreen-eager-escape')).toHaveAttribute('data-hydrated', '');
    await expect(page.locator('#offscreen-eager-escape')).toHaveAttribute('w-hydrate', 'eager');
    await page.locator('#offscreen-eager-escape button').click();
    await expect(page.locator('#offscreen-eager-escape .count')).toHaveText('1');
  });
});
