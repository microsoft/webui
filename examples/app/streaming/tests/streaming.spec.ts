// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { test, expect, type Page } from '@playwright/test';

/**
 * Progressive streaming hydration coverage for this example's
 * composer / weather / feed priority ordering, built on the Phase 1
 * boundary contract in DESIGN.md ("Progressive Streaming Hydration —
 * Phase 1").
 *
 * Boundary sequence numbers follow document order:
 *
 * | Sequence | Boundary        | Gap before the *next* flush        |
 * | -------- | --------------- | ---------------------------------- |
 * | 0        | weather shell   | none — the composer must not wait   |
 * | 1        | composer        | jittered 500-1000ms                 |
 * | 2        | feed batch 1    | jittered 500-1000ms                 |
 * | 3        | feed batch 2    | jittered 500-1000ms                 |
 * | 4        | feed batch 3    | none — the response closes promptly |
 *
 * The server (`server/src/main.rs`) paces only the gaps that precede feed
 * batches, bounded by `--feed-delay-min-ms` / `--feed-delay-max-ms`, so
 * delivery order is observable over real network timing. These tests never
 * sleep to assert ordering: they capture the client coordinator's own
 * `webui:boundary-hydrated` / `webui:hydration-complete` events
 * (`@microsoft/webui-framework`'s `streaming.ts` / `lifecycle.ts`) via
 * `page.addInitScript`, then use `page.waitForFunction` to resolve the
 * instant a condition becomes true.
 */

interface BoundaryEvent {
  sequence: number;
  terminal: boolean;
  t: number;
}

declare global {
  interface Window {
    __WEBUI_STREAMING_DEBUG__: boolean;
    __boundaryEvents: BoundaryEvent[];
    __dclFired: boolean;
    __hydrationCompleteFired: boolean;
    __hydrationCompleteTime: number;
  }
}

/** Install test-only capture globals before any page script runs. Kept out
 *  of the production example so `src/index.ts` stays a faithful sample. */
async function instrumentPage(page: Page): Promise<void> {
  await page.addInitScript(() => {
    // Per-boundary `webui:boundary-hydrated` events are opt-in diagnostics
    // (the coordinator skips allocating a CustomEvent per boundary unless this
    // flag is set). Enable it before installing the listeners below so the
    // tests can observe delivery ordering. The production example leaves it
    // off by default.
    window.__WEBUI_STREAMING_DEBUG__ = true;

    window.__boundaryEvents = [];
    window.__dclFired = false;
    window.__hydrationCompleteFired = false;
    window.__hydrationCompleteTime = -1;

    window.addEventListener('webui:boundary-hydrated', (event) => {
      const detail = (event as CustomEvent<{ sequence: number; terminal: boolean }>).detail;
      window.__boundaryEvents.push({ ...detail, t: performance.now() });
    });
    window.addEventListener('webui:hydration-complete', () => {
      window.__hydrationCompleteFired = true;
      window.__hydrationCompleteTime = performance.now();
    });
    document.addEventListener('DOMContentLoaded', () => {
      window.__dclFired = true;
    });
  });
}

function boundaryEvents(page: Page): Promise<BoundaryEvent[]> {
  return page.evaluate(() => window.__boundaryEvents);
}

function hasSequence(page: Page, sequence: number): Promise<boolean> {
  return page.evaluate((seq) => window.__boundaryEvents.some((e) => e.sequence === seq), sequence);
}

test.describe('streaming priority hydration', () => {
  test('composer paints and becomes interactive before DOMContentLoaded', async ({ page }) => {
    await instrumentPage(page);
    // 'commit' resolves as soon as the navigation's response headers are
    // received — well before the paced response body finishes — so this
    // test can observe the still-open-response state instead of racing it.
    await page.goto('/', { waitUntil: 'commit' });

    await page.waitForFunction(() => window.__boundaryEvents.some((e) => e.sequence === 1));
    expect(await page.evaluate(() => window.__dclFired)).toBe(false);

    const input = page.locator('message-composer input.composer-input');
    await expect(input).toBeVisible();
    await input.fill('Hello before the feed even arrives');
    await page.locator('message-composer button.composer-submit').click();
    await expect(page.locator('message-composer [data-testid="composer-posted"]')).toHaveText(
      'Posted: Hello before the feed even arrives',
    );

    // The interaction above never touched the network, so the document is
    // still open at this point too — DOMContentLoaded needs the whole
    // (still-paced) response to finish parsing.
    expect(await page.evaluate(() => window.__dclFired)).toBe(false);

    await page.waitForLoadState('domcontentloaded');
    expect(await page.evaluate(() => window.__dclFired)).toBe(true);
  });

  test('feed batch 1 hydrates and is interactive before batch 2 is delivered', async ({ page }) => {
    await instrumentPage(page);
    await page.goto('/', { waitUntil: 'commit' });

    await page.waitForFunction(() => window.__boundaryEvents.some((e) => e.sequence === 2));
    expect(await hasSequence(page, 3)).toBe(false);

    const firstItem = page.locator('feed-item[post-id="1"]');
    const likeButton = firstItem.locator('[data-testid="feed-item-like"]');
    const likeCount = firstItem.locator('[data-testid="feed-item-like-count"]');
    await expect(likeCount).toHaveText('4');
    await likeButton.click();
    await expect(likeCount).toHaveText('5');

    // Still true after interacting: batch 2 has not been delivered yet.
    expect(await hasSequence(page, 3)).toBe(false);
  });

  test('all three feed batches hydrate in order and stay independently interactive', async ({ page }) => {
    await instrumentPage(page);
    await page.goto('/');
    await page.waitForFunction(() => window.__hydrationCompleteFired);

    const events = await boundaryEvents(page);
    const sequences = events.map((e) => e.sequence);
    expect(sequences).toEqual(expect.arrayContaining([0, 1, 2, 3, 4]));
    for (let i = 1; i < sequences.length; i++) {
      expect(sequences[i]).toBeGreaterThan(sequences[i - 1]);
    }

    const terminalEvents = events.filter((e) => e.terminal);
    expect(terminalEvents).toHaveLength(1);
    expect(events[events.length - 1].terminal).toBe(true);

    // Each batch's item is independently interactive, and clicking one
    // item's like button never mutates another chunk's state — proving
    // the feed container itself was never hydrated as a whole.
    const items: Array<[postId: string, startCount: string]> = [
      ['1', '4'],
      ['2', '1'],
      ['3', '9'],
      ['4', '0'],
    ];
    for (const [postId, startCount] of items) {
      const item = page.locator(`feed-item[post-id="${postId}"]`);
      await expect(item.locator('[data-testid="feed-item-like-count"]')).toHaveText(startCount);
    }

    await page.locator('feed-item[post-id="3"] [data-testid="feed-item-like"]').click();
    await expect(page.locator('feed-item[post-id="3"] [data-testid="feed-item-like-count"]')).toHaveText('10');

    // Unrelated items are untouched by batch 2's click.
    for (const [postId, startCount] of items) {
      if (postId === '3') continue;
      await expect(
        page.locator(`feed-item[post-id="${postId}"] [data-testid="feed-item-like-count"]`),
      ).toHaveText(startCount);
    }
  });

  test('the weather panel hydrates first and resolves independently of the stream', async ({ page }) => {
    await instrumentPage(page);
    await page.goto('/', { waitUntil: 'commit' });

    // The weather shell is boundary 0: it carries no server data, so it
    // commits immediately and must never delay the composer behind it.
    await page.waitForFunction(() => window.__boundaryEvents.some((e) => e.sequence === 0));

    const panel = page.locator('weather-panel [data-testid="weather-panel"]');
    await expect(panel).toHaveAttribute('data-status', 'loading');
    await expect(page.locator('weather-panel [data-testid="weather-skeleton"]')).toBeVisible();

    // The forecast endpoint is deliberately slower than a feed gap, so this
    // resolves through the component's own fetch rather than the stream.
    const summary = page.locator('weather-panel [data-testid="weather-summary"]');
    await expect(summary).toBeVisible({ timeout: 15_000 });
    await expect(panel).toHaveAttribute('data-status', 'ready');
    await expect(page.locator('weather-panel [data-testid="weather-temperature"]')).not.toBeEmpty();
    await expect(page.locator('weather-panel [data-testid="weather-condition"]')).not.toBeEmpty();

    // The skeleton branch is torn down, not merely hidden.
    await expect(page.locator('weather-panel [data-testid="weather-skeleton"]')).toHaveCount(0);
  });

  test('the weather island loads its own module from inside its boundary', async ({ page }) => {
    const requests: { url: string; t: number }[] = [];
    const start = Date.now();
    page.on('request', (request) => {
      if (request.resourceType() === 'script') {
        requests.push({ url: new URL(request.url()).pathname, t: Date.now() - start });
      }
    });

    await instrumentPage(page);
    await page.goto('/');
    await page.waitForFunction(() => window.__hydrationCompleteFired);

    // The island is a separate entry point: its code must never be inside
    // the critical bundle, or splitting it bought nothing.
    const critical = await page.evaluate(async () => {
      const source = await (await fetch('./index.js')).text();
      return { bytes: source.length, mentionsPanel: source.includes('weather-panel') };
    });
    expect(critical.mentionsPanel).toBe(false);

    // It is fetched because the boundary's own <script> reached the parser,
    // so it is requested after the critical entry rather than alongside it.
    const entry = requests.find((r) => r.url.endsWith('/index.js'));
    const island = requests.find((r) => r.url.endsWith('/weather-panel.js'));
    expect(entry).toBeDefined();
    expect(island).toBeDefined();
    expect(island!.t).toBeGreaterThanOrEqual(entry!.t);

    // Nothing preloads the island: its whole point is to stay off the
    // critical path that moving it out of index.js just cleared.
    const preloads = await page.evaluate(() =>
      Array.from(document.head.querySelectorAll<HTMLLinkElement>('link[rel="modulepreload"]')).map(
        (link) => new URL(link.href).pathname,
      ),
    );
    expect(preloads.some((href) => href.endsWith('/weather-panel.js'))).toBe(false);

    // Arriving late is safe: the boundary commits before the class exists,
    // so the coordinator parks the root and activates it on definition.
    await expect(page.locator('weather-panel [data-testid="weather-panel"]')).toHaveCount(1);
    await expect(page.locator('weather-panel')).not.toHaveAttribute('data-ws', /.*/);
  });

  test('webui:hydration-complete fires only after the terminal boundary record', async ({ page }) => {
    await instrumentPage(page);
    await page.goto('/');
    await page.waitForFunction(() => window.__hydrationCompleteFired);

    const events = await boundaryEvents(page);
    const terminalEvent = events.find((e) => e.terminal);
    expect(terminalEvent).toBeDefined();

    const hydrationCompleteTime = await page.evaluate(() => window.__hydrationCompleteTime);
    expect(hydrationCompleteTime).toBeGreaterThanOrEqual(terminalEvent!.t);
  });

  test('no boundary scaffolding remains once hydration completes', async ({ page }) => {
    await instrumentPage(page);
    await page.goto('/');
    await page.waitForFunction(() => window.__hydrationCompleteFired);

    const leftovers = await page.evaluate(() => {
      const scripts = document.querySelectorAll('script[data-webui-boundary]').length;
      const sentinels = document.querySelectorAll('webui-hydrate').length;
      // `<if>` conditions compile to an inline `templateFns` script emitted
      // between each payload and its sentinel. Those are boundary
      // scaffolding too, so a non-zero count means
      // `removeBoundaryScaffolding` missed them. The weather island's own
      // loader also lives in <body>, inside its boundary, so it is excluded
      // by src — teardown must not treat authored content as scaffolding.
      const bodyScripts = Array.from(document.body.querySelectorAll('script')).filter(
        (script) => !script.src.endsWith('/weather-panel.js'),
      ).length;

      const walker = document.createTreeWalker(document.documentElement, NodeFilter.SHOW_COMMENT);
      let markers = 0;
      let node: Node | null;
      while ((node = walker.nextNode())) {
        if (/^\/?wb:\d+$/.test((node as Comment).data)) markers++;
      }

      // The island loader is authored markup, not scaffolding: it must
      // survive the teardown that removes everything else above.
      const islandLoaders = document.querySelectorAll(
        'script[src$="/weather-panel.js"]',
      ).length;

      return { scripts, sentinels, markers, bodyScripts, islandLoaders };
    });

    expect(leftovers).toEqual({
      scripts: 0,
      sentinels: 0,
      markers: 0,
      bodyScripts: 0,
      islandLoaders: 1,
    });
  });

  test('the streaming response eventually closes', async ({ page }) => {
    await instrumentPage(page);
    await page.goto('/');
    await page.waitForLoadState('load');

    // `responseEnd` is only populated once the full response body has been
    // received, so a non-zero value proves the paced stream actually ended
    // rather than staying open indefinitely.
    const responseEnd = await page.evaluate(() => {
      const [nav] = performance.getEntriesByType('navigation') as PerformanceNavigationTiming[];
      return nav?.responseEnd ?? 0;
    });
    expect(responseEnd).toBeGreaterThan(0);
  });
});
