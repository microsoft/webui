// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { test, expect, type Page } from '@playwright/test';

/**
 * Progressive streaming hydration coverage for this example's
 * composer / weather / feed priority ordering, built on the boundary
 * contract in DESIGN.md ("Progressive Streaming Hydration").
 *
 * Runtime boundary IDs follow discovery order inside `<streaming-page>`.
 * Response record sequences also include state updates, the page component's
 * span completion, and the terminal record:
 *
 * | Boundary ID | Boundary      | Delivery                            |
 * | ----------- | ------------- | ----------------------------------- |
 * | 0           | weather shell | immediate, then server-updatable     |
 * | 1           | composer      | immediate                            |
 * | 2           | feed batch 1  | jittered 500-1000ms                  |
 * | 3           | feed batch 2  | jittered 500-1000ms                  |
 * | 4           | feed batch 3  | jittered 500-1000ms                  |
 *
 * In the full controlled release, the weather state update consumes response
 * sequence 3 between feed batch 1 and feed batch 2. The page span completion
 * follows the final checkpoint, then the terminal closes the response.
 *
 * The Node API (`server/src/pacing.ts`) paces only the gaps that precede feed
 * batches, bounded by `--feed-delay-min-ms` / `--feed-delay-max-ms`, so
 * delivery order is observable over real network timing. A weather state
 * record may consume a response sequence between any two checkpoints. These tests never
 * sleep to assert ordering: they capture the client coordinator's own
 * `webui:boundary-hydrated` / `webui:hydration-complete` events
 * (`@microsoft/webui-framework`'s `streaming.ts` / `lifecycle.ts`) via
 * `page.addInitScript`, then use `page.waitForFunction` to resolve the
 * instant a condition becomes true.
 */

interface BoundaryEvent {
  sequence: number;
  terminal: boolean;
  kind: 'checkpoint' | 'span' | 'update' | 'terminal';
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

let sessionCounter = 0;
const controlledSessions = new WeakMap<Page, string>();

async function gotoControlled(
  page: Page,
  waitUntil: 'commit' | 'domcontentloaded' | 'load' = 'commit',
): Promise<string> {
  const session = `worker-${process.pid}-${sessionCounter++}`;
  controlledSessions.set(page, session);
  await page.goto(`/?test=${session}`, { waitUntil });
  return session;
}

async function release(
  page: Page,
  session: string,
  gate: 'feed' | 'weather' | 'all',
): Promise<void> {
  const response = await page.request.post(`/api/__test/${session}/${gate}`);
  expect(response.ok(), `release ${gate} for ${session}`).toBe(true);
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
      const detail = (event as CustomEvent<Omit<BoundaryEvent, 't'>>).detail;
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
  test.afterEach(async ({ page }) => {
    const session = controlledSessions.get(page);
    if (session) await release(page, session, 'all');
  });

  test('identifies the streaming home page', async ({ page }) => {
    await page.goto('/');

    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Streaming Home');
  });

  test('composer paints and becomes interactive before DOMContentLoaded', async ({ page }) => {
    await instrumentPage(page);
    // 'commit' resolves as soon as the navigation's response headers are
    // received — well before the paced response body finishes — so this
    // test can observe the still-open-response state instead of racing it.
    const session = await gotoControlled(page);

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
    await expect(page.getByTestId('page-tail')).toHaveCount(0);

    await release(page, session, 'all');
    await page.waitForLoadState('domcontentloaded');
    expect(await page.evaluate(() => window.__dclFired)).toBe(true);
    await expect(page.getByTestId('page-tail')).toBeVisible();
  });

  test('feed batch 1 hydrates and is interactive before batch 2 is delivered', async ({ page }) => {
    await instrumentPage(page);
    const session = await gotoControlled(page);

    await release(page, session, 'feed');
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
    const session = await gotoControlled(page);
    await release(page, session, 'all');
    await page.waitForFunction(() => window.__hydrationCompleteFired);

    const events = await boundaryEvents(page);
    const sequences = events.map((e) => e.sequence);
    expect(events.filter((event) => event.kind === 'checkpoint')).toHaveLength(5);
    expect(events.filter((event) => event.kind === 'span')).toHaveLength(1);
    expect(events.filter((event) => event.kind === 'update')).toHaveLength(1);
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

  test('weather state arrives between feed chunks on the original response', async ({ page }) => {
    const requests: string[] = [];
    page.on('request', (request) => requests.push(new URL(request.url()).pathname));
    await instrumentPage(page);
    const session = await gotoControlled(page);

    // The weather shell is boundary 0: it carries no server data, so it
    // commits immediately and must never delay the composer behind it.
    await page.waitForFunction(() => window.__boundaryEvents.some((e) => e.sequence === 0));

    const panel = page.locator('weather-panel [data-testid="weather-panel"]');
    await expect(panel).toHaveAttribute('data-status', 'loading');
    await expect(page.locator('weather-panel [data-testid="weather-skeleton"]')).toBeVisible();

    await release(page, session, 'feed');
    await expect(page.locator('feed-item[post-id="1"]')).not.toHaveAttribute('data-ws', /.*/);
    await expect(page.locator('feed-item[post-id="3"]')).toHaveCount(0);

    // Forecast readiness wins the race for the next response record, while
    // feed batch 2 remains gated.
    const summary = page.locator('weather-panel [data-testid="weather-summary"]');
    await release(page, session, 'weather');
    await expect(summary).toBeVisible({ timeout: 15_000 });
    await expect(panel).toHaveAttribute('data-status', 'ready');
    await expect(page.locator('weather-panel [data-testid="weather-temperature"]')).not.toBeEmpty();
    await expect(page.locator('weather-panel [data-testid="weather-condition"]')).not.toBeEmpty();
    await expect(page.locator('feed-item[post-id="3"]')).toHaveCount(0);
    expect(await page.evaluate(() => window.__dclFired)).toBe(false);
    expect(requests.some((path) => path.endsWith('/api/weather'))).toBe(false);

    // The skeleton branch is torn down, not merely hidden.
    await expect(page.locator('weather-panel [data-testid="weather-skeleton"]')).toHaveCount(0);
  });

  test('applies a weather update that arrives before the deferred island activates', async ({ page }) => {
    await page.route('**/weather-panel.js', async (route) => {
      const response = await route.fetch();
      await new Promise((resolve) => setTimeout(resolve, 250));
      await route.fulfill({ response });
    });
    const session = await gotoControlled(page);
    await release(page, session, 'all');

    await expect(page.locator('weather-panel [data-testid="weather-summary"]')).toBeVisible({
      timeout: 15_000,
    });
    await expect(page.locator('weather-panel [data-testid="weather-panel"]')).toHaveAttribute(
      'data-status',
      'ready',
    );
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
    const session = await gotoControlled(page);
    await release(page, session, 'all');
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

  test('the compiler preloads the critical entry shared chunks, largest first', async ({
    page,
  }) => {
    await instrumentPage(page);
    const session = await gotoControlled(page);
    await release(page, session, 'all');
    await page.waitForFunction(() => window.__hydrationCompleteFired);

    const preloads = await page.evaluate(() =>
      Array.from(document.head.querySelectorAll<HTMLLinkElement>('link[rel="modulepreload"]')).map(
        (link) => new URL(link.href).pathname,
      ),
    );

    // The shared framework chunk is a *static* import of index.js, so the
    // preload scanner cannot discover it until index.js has downloaded and
    // parsed. That waterfall is what the hint removes.
    expect(preloads.length).toBeGreaterThan(0);
    for (const href of preloads) {
      expect(href).toMatch(/\/chunk-[^/]+\.js$/);
    }

    // Order is the whole feature. Preloads are issued in document order over
    // one connection, so a small chunk listed first delays the long pole and
    // gives back the entire win (a measured 125 ms swing).
    const sizes = await page.evaluate(
      async (paths) =>
        Promise.all(
          paths.map(async (path) => (await (await fetch(path)).text()).length),
        ),
      preloads,
    );
    const descending = [...sizes].sort((a, b) => b - a);
    expect(sizes).toEqual(descending);
  });

  test('webui:hydration-complete fires only after the terminal boundary record', async ({ page }) => {
    await instrumentPage(page);
    const session = await gotoControlled(page);
    await release(page, session, 'all');
    await page.waitForFunction(() => window.__hydrationCompleteFired);

    const events = await boundaryEvents(page);
    const terminalEvent = events.find((e) => e.terminal);
    expect(terminalEvent).toBeDefined();

    const hydrationCompleteTime = await page.evaluate(() => window.__hydrationCompleteTime);
    expect(hydrationCompleteTime).toBeGreaterThanOrEqual(terminalEvent!.t);
  });

  test('no boundary scaffolding remains once hydration completes', async ({ page }) => {
    await instrumentPage(page);
    const session = await gotoControlled(page);
    await release(page, session, 'all');
    await page.waitForFunction(() => window.__hydrationCompleteFired);

    const leftovers = await page.evaluate(() => {
      const pageRoot = document.querySelector('streaming-page')?.shadowRoot;
      const roots: ParentNode[] = pageRoot ? [document, pageRoot] : [document];
      const scripts = roots.reduce(
        (count, root) => count + root.querySelectorAll('script[data-webui-boundary]').length,
        0,
      );
      const sentinels = roots.reduce(
        (count, root) => count + root.querySelectorAll('webui-hydrate').length,
        0,
      );
      // `<if>` conditions compile to an inline `templateFns` script emitted
      // between each payload and its sentinel. Those are boundary
      // scaffolding too, so a non-zero count means
      // `removeBoundaryScaffolding` missed them. The weather island's own
      // loader also lives in <body>, inside its boundary, so it is excluded
      // by src — teardown must not treat authored content as scaffolding.
      const bodyScripts = Array.from(pageRoot?.querySelectorAll('script') ?? []).filter(
        (script) => !script.src.endsWith('/weather-panel.js'),
      ).length;

      let markers = 0;
      const markerRoots: Node[] = pageRoot
        ? [document.documentElement, pageRoot]
        : [document.documentElement];
      for (const root of markerRoots) {
        const walker = document.createTreeWalker(root, NodeFilter.SHOW_COMMENT);
        let node: Node | null;
        while ((node = walker.nextNode())) {
          if (/^\/?w[bs]:\d+$/.test((node as Comment).data)) markers++;
        }
      }

      // The island loader is authored markup, not scaffolding: it must
      // survive the teardown that removes everything else above.
      const islandLoaders = pageRoot?.querySelectorAll(
        'script[src$="/weather-panel.js"]',
      ).length ?? 0;

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
    const session = await gotoControlled(page);
    await release(page, session, 'all');
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

/**
 * Reload recovery.
 *
 * A reload is the hardest case for streaming hydration: the previous stream is
 * aborted mid-flight, the module graph is already warm in the HTTP cache, and a
 * fresh document begins committing boundaries before its islands are defined.
 * That reorders activation against delivery in ways a cold first load never
 * does, which is exactly where a boundary can be left holding a root that never
 * came alive.
 *
 * These tests assert the user-visible contract rather than internals: after the
 * dust settles the weather island is `ready`, its skeleton branch is gone, the
 * feed is fully painted, no boundary scaffolding survives, and nothing threw.
 */
test.describe('streaming reload recovery', () => {
  // Every session this block opens, so teardown can release all of them. A
  // gated session whose stream is abandoned mid-flight keeps its slot against
  // `--max-concurrent-streams` until something releases its gates, so leaving
  // even one behind slowly starves the shared test API.
  const openedSessions = new WeakMap<Page, Set<string>>();

  function trackSession(page: Page, session: string): string {
    let sessions = openedSessions.get(page);
    if (!sessions) {
      sessions = new Set<string>();
      openedSessions.set(page, sessions);
    }
    sessions.add(session);
    return session;
  }

  test.afterEach(async ({ page }) => {
    for (const session of openedSessions.get(page) ?? []) {
      // A session the API never created 404s here, which is fine: nothing to
      // release. Teardown must not fail the test it is cleaning up after.
      await page.request.post(`/api/__test/${session}/all`).catch(() => undefined);
    }
  });

  /** Fail loudly on any uncaught error or console error the page reports. */
  function trackPageErrors(page: Page): string[] {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(`pageerror: ${error.message}`));
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(`console: ${message.text()}`);
    });
    return errors;
  }

  /**
   * Navigate and resolve once the document's stream is provably underway.
   *
   * `waitUntil: 'commit'` resolves on the CLI's response headers, which the CLI
   * can emit before the API has seen the request and created the test session.
   * Releasing a gate at that point races session creation and 404s. The
   * composer ships in an immediate, ungated boundary, so its presence is proof
   * the API is streaming this document and the session exists.
   */
  async function gotoStreaming(page: Page, session: string): Promise<void> {
    trackSession(page, session);
    await page.goto(`/?test=${session}`, { waitUntil: 'commit' });
    await page.locator('message-composer').waitFor({ state: 'attached', timeout: 15_000 });
  }

  /** The full post-hydration contract for a healthy streamed document. */
  async function expectFullyHydrated(page: Page): Promise<void> {
    const panel = page.locator('weather-panel [data-testid="weather-panel"]');
    await expect(panel).toHaveAttribute('data-status', 'ready', { timeout: 15_000 });
    await expect(page.locator('weather-panel [data-testid="weather-summary"]')).toBeVisible();
    await expect(page.locator('weather-panel [data-testid="weather-temperature"]')).not.toBeEmpty();
    // The skeleton is torn down, not hidden: a surviving skeleton means the
    // island re-rendered the branch the server had already replaced.
    await expect(page.locator('weather-panel [data-testid="weather-skeleton"]')).toHaveCount(0);

    // Every feed batch landed, so no checkpoint was dropped by the reload.
    await expect(page.locator('feed-item')).toHaveCount(4);

    // `data-ws` is the coordinator's work marker. Its absence here means every
    // root it marked was resolved -- activated, or explicitly abandoned --
    // rather than left parked forever waiting on a stream that is gone.
    await expect(page.locator('[data-ws]')).toHaveCount(0);
    await expect(page.locator('webui-hydrate')).toHaveCount(0);
    await expect(page.locator('script[data-webui-boundary]')).toHaveCount(0);
  }

  // The island's module can arrive before, during, or after its boundary
  // commits. The bug this locks in only appeared inside a narrow timing band,
  // so sweep across it instead of trusting one delay to land there.
  for (const delayMs of [0, 60, 200, 400]) {
    test(`reloads cleanly with the island module delayed ${delayMs}ms`, async ({ page }) => {
      const errors = trackPageErrors(page);
      await page.route('**/weather-panel*.js', async (route) => {
        const response = await route.fetch();
        if (delayMs > 0) await new Promise((resolve) => setTimeout(resolve, delayMs));
        await route.fulfill({ response });
      });

      const first = `band-${delayMs}-${process.pid}-${sessionCounter++}`;
      await gotoStreaming(page, first);
      await release(page, first, 'all');
      await expectFullyHydrated(page);

      // Reload onto a fresh session so the second document is paced like a
      // real one instead of replaying an already-drained session instantly.
      await gotoStreaming(page, `${first}-r`);
      await release(page, `${first}-r`, 'all');
      await expectFullyHydrated(page);

      expect(errors, 'reload produced page errors').toEqual([]);
    });
  }

  test('recovers when a reload interrupts a stream that is still open', async ({ page }) => {
    const errors = trackPageErrors(page);
    await instrumentPage(page);

    // Commit the first document and let only feed batch 1 through, so the
    // response is provably still open when the reload aborts it.
    const first = `interrupt-${process.pid}-${sessionCounter++}`;
    await gotoStreaming(page, first);
    await release(page, first, 'feed');
    await page.waitForFunction(() => window.__boundaryEvents.some((e) => e.sequence === 2));
    expect(await page.evaluate(() => window.__hydrationCompleteFired)).toBe(false);

    await gotoStreaming(page, `${first}-r`);
    await release(page, `${first}-r`, 'all');

    await page.waitForFunction(() => window.__hydrationCompleteFired, undefined, {
      timeout: 15_000,
    });
    await expectFullyHydrated(page);
    expect(errors, 'interrupted reload produced page errors').toEqual([]);
  });

  test('survives rapid consecutive reloads', async ({ page }) => {
    const errors = trackPageErrors(page);
    const base = `rapid-${process.pid}-${sessionCounter++}`;

    // Navigate away repeatedly without ever letting a document finish. Each
    // abort leaves the coordinator holding parked roots for a document that
    // is being torn down -- the sequence behind the original blank-page report.
    for (let index = 0; index < 5; index++) {
      const session = trackSession(page, `${base}-${index}`);
      await page.goto(`/?test=${session}`, { waitUntil: 'commit' });
      // Open this document's gates immediately. The browser has already moved
      // on, so nothing here changes what the client sees -- it just lets the
      // orphaned server stream run to completion and give its slot back,
      // instead of parking on a gate no one will ever open. A real server
      // paces on timers, so it drains this way on its own.
      await page.request.post(`/api/__test/${session}/all`).catch(() => undefined);
    }

    await gotoStreaming(page, `${base}-final`);
    await release(page, `${base}-final`, 'all');

    await expectFullyHydrated(page);
    expect(errors, 'rapid reloads produced page errors').toEqual([]);
  });

  test('renders the app when the API refuses to open a stream', async ({ page }) => {
    const errors = trackPageErrors(page);

    // `?refuse=1` returns the identical 503 a saturated server sends. The CLI
    // must degrade to a rendered page rather than proxying the upstream error
    // body, which would surface as a text/plain error where the app should be.
    const response = await page.goto('/?refuse=1');
    expect(response?.status()).toBe(200);
    expect(response?.headers()['content-type']).toContain('text/html');

    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Streaming Home');
    expect(await page.content()).not.toContain('streaming render capacity');

    // The critical bundle still hydrates, so the shell is genuinely usable and
    // not just static markup that happens to parse.
    const input = page.locator('message-composer input.composer-input');
    await expect(input).toBeVisible();
    await input.fill('Usable even without a stream');
    await page.locator('message-composer button.composer-submit').click();
    await expect(page.locator('message-composer [data-testid="composer-posted"]')).toHaveText(
      'Posted: Usable even without a stream',
    );

    // Degrading means no server data arrived, so the island stays in its
    // loading branch. That is the honest state -- not an error, and not
    // fabricated content.
    await expect(page.locator('weather-panel [data-testid="weather-panel"]')).toHaveAttribute(
      'data-status',
      'loading',
    );
    await expect(page.locator('feed-item')).toHaveCount(0);
    expect(errors, 'the degraded page produced errors').toEqual([]);
  });
});
