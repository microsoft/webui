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
    // Only lazy roots should still be inert here — the `w-hydrate="eager"`
    // instance mounts synchronously regardless of visibility and is excluded.
    await expect(
      page.locator('[data-hydrated]:not(#offscreen-eager-escape)'),
    ).toHaveCount(0);
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

  test('activates nested lazy components parent-first', async ({ page }) => {
    await page.goto(FIXTURE);

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

  test('rehydrates after a mounted component is destroyed while detached', async ({ page }) => {
    await page.goto(FIXTURE);
    await expect(page.locator('#visible')).toHaveAttribute('data-hydrated', '');

    await page.locator('#visible').evaluate(async (element) => {
      const parent = element.parentNode;
      element.remove();
      await Promise.resolve();
      parent?.appendChild(element);
    });
    await page.locator('#visible button').evaluate((button) => {
      (button as HTMLElement).click();
    });

    await expect(page.locator('#visible .count')).toHaveText('1');
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
      warnings.filter((warning) => warning.includes('visible-hydration.js')),
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
});

test.describe('w-hydrate="eager" SSR escape hatch', () => {
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
