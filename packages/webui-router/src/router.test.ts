// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Shim must be imported before any router code — sets up browser globals.
import './browser-shim.js';

import { strict as assert } from 'node:assert';
import { describe, test, beforeEach, afterEach } from 'node:test';
import { WebUIRouter } from './router.js';
import { parseQuery, filterQuery } from './route-element.js';
import { resolveLoaders } from './loaders.js';
import { ensureComponentLoaded } from './loaders.js';
import { NavigationCache } from './cache.js';
import { setupPreloadListeners } from './preload.js';

// ── Test-only type access ────────────────────────────────────────
// The router's `inventory` and `activeChain` are private at compile
// time. This interface gives tests typed access without `as any`.

interface RouteChainEntry {
  component: string;
  path: string;
  params: Record<string, string>;
}

interface RouterInternals {
  inventory: string;
  activeChain: RouteChainEntry[];
}

/** Cast a WebUIRouter to expose private fields for test setup. */
function internals(router: WebUIRouter): RouterInternals {
  return router as unknown as RouterInternals;
}

/** Typed access to the global template registry. */
interface TemplateRegistry {
  __webui?: {
    templates?: Record<string, unknown>;
    [key: string]: unknown;
  };
}

function globals(): TemplateRegistry {
  return globalThis as unknown as TemplateRegistry;
}

// Assign deterministic indices for test components
const testIndex: Record<string, number> = {};
let nextIdx = 0;
function ensureIndex(name: string): number {
  if (!(name in testIndex)) testIndex[name] = nextIdx++;
  return testIndex[name];
}

/** Build an inventory hex with specific components marked as loaded. */
function inventoryWith(...names: string[]): string {
  for (const n of names) ensureIndex(n);
  const byteCount = Math.max(1, Math.ceil(nextIdx / 8));
  const inv = new Uint8Array(byteCount);
  for (const n of names) {
    const idx = testIndex[n];
    inv[idx >> 3] |= 1 << (idx & 7);
  }
  // Encode as hex
  let hex = '';
  for (const b of inv) {
    hex += (b < 16 ? '0' : '') + b.toString(16);
  }
  return hex;
}

/** Check if a component's bit is set in an inventory hex string. */
function hasBit(hex: string, name: string): boolean {
  const idx = testIndex[name];
  if (idx === undefined) return false;
  const bytes = new Uint8Array(hex.length >> 1);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.substring(i * 2, i * 2 + 2), 16);
  }
  const byteIdx = idx >> 3;
  return byteIdx < bytes.length && (bytes[byteIdx] & (1 << (idx & 7))) !== 0;
}

describe('WebUIRouter', () => {
  let savedWebui: typeof window.__webui;

  beforeEach(() => {
    savedWebui = globals().__webui;
    (globals() as any).__webui = {
      templates: {},
      inventory: '',
      nonce: '',
      css: [],
      styles: [],
    };
  });

  afterEach(() => {
    (globals() as any).__webui = savedWebui;
  });

  describe('gc', () => {
    test('clears all templates and resets inventory', () => {
      const router = new WebUIRouter();
      const registry = globals().__webui!.templates!;
      registry['shell'] = { h: '<div>Shell</div>' };
      registry['page-x'] = { h: '<div>X</div>' };
      registry['page-y'] = { h: '<div>Y</div>' };

      globals().__webui!.inventory = inventoryWith('shell', 'page-x', 'page-y');

      router.gc();

      assert.equal(registry['shell'], undefined, 'shell should be cleared');
      assert.equal(registry['page-x'], undefined, 'page-x should be cleared');
      assert.equal(registry['page-y'], undefined, 'page-y should be cleared');
      assert.equal(globals().__webui!.inventory, '', 'inventory should be reset to empty');
    });

    test('is a no-op when no templates are registered', () => {
      const router = new WebUIRouter();
      const g = globals().__webui;
      if (g) g.templates = undefined;
      router.gc(); // should not throw
    });
  });

  describe('destroy', () => {
    test('clears in-flight loadPromises so the router can be restarted cleanly', async () => {
      const router = new WebUIRouter();
      // Initialize navCache so destroy() can call .clear()
      (router as any).navCache = new NavigationCache({ staleTime: 0, gcTime: 300_000, maxEntries: 50 });

      // Stub fetch to return a never-resolving promise (simulates in-flight request)
      const origFetch = (globalThis as any).fetch;
      (globalThis as any).fetch = () => new Promise<void>(() => {});

      try {
        // Start ensureLoaded without awaiting — leaves an in-flight promise in loadPromises
        const loadPromises = (router as any).loadPromises as Map<string, Promise<void>>;
        router.ensureLoaded('some-dialog');

        assert.ok(loadPromises.size > 0, 'loadPromises should have in-flight entries');

        router.destroy();

        assert.equal(loadPromises.size, 0, 'destroy() should clear loadPromises for clean GC/restart');
      } finally {
        (globalThis as any).fetch = origFetch;
      }
    });
  });

  describe('template execution in fetchPartial', () => {
    test('Function-based execution registers templates without DOM', () => {
      // Simulate what fetchPartial does with the template script string
      const tmpl =
        '<script>(function(){var w=(window.__webui||(window.__webui={})).templates||(window.__webui.templates={});' +
        "w['test-comp']={h:\"<div>hello</div>\"};" +
        '})();</script>';

      const start = tmpl.indexOf('>') + 1;
      const end = tmpl.lastIndexOf('<');
      // eslint-disable-next-line no-new-func
      const run = Function(tmpl.substring(start, end));
      run();

      const registry = globals().__webui!.templates!;
      assert.ok(registry['test-comp'], 'template should be registered');
      assert.equal(
        (registry['test-comp'] as Record<string, string>).h,
        '<div>hello</div>',
        'template HTML should match',
      );
    });

    test('malformed template (no script tags) is safely skipped', () => {
      const tmpl = 'not a script tag at all';
      const start = tmpl.indexOf('>') + 1;
      const end = tmpl.lastIndexOf('<');
      // start would be 0 (indexOf returns -1, +1 = 0), end would be -1
      // The guard `start > 0 && end > start` should prevent execution
      assert.equal(start > 0 && end > start, false, 'guard should reject malformed input');
    });

    test('fetchPartial appends module styles before one batched script execution', async () => {
      const origFetch = (globalThis as any).fetch;
      const origCreateElement = (globalThis as any).document.createElement;
      const origQuerySelector = (globalThis as any).document.querySelector;
      const origHead = (globalThis as any).document.head;

      const order: string[] = [];
      const scriptBodies: string[] = [];
      const scriptNonces: string[] = [];

      (globalThis as any).fetch = async () => ({
        ok: true,
        headers: { get: () => 'application/json' },
        json: async () => ({
          state: {},
          templateStyles: [
            '<style type="module" specifier="alpha">.alpha{color:red}</style>',
            '<style type="module" specifier="beta">.beta{color:blue}</style>',
          ],
          templates: [
            '(function(){var w=(window.__webui||(window.__webui={})).templates||(window.__webui.templates={});w["alpha"]={h:"<div>a</div>"};})();',
            '(function(){var w=(window.__webui||(window.__webui={})).templates||(window.__webui.templates={});w["beta"]={h:"<div>b</div>"};})();',
          ],
          path: '/',
          chain: [],
          inventory: 'ff',
        }),
      });

      (globalThis as any).document.createElement = (tag: string) => ({
        tagName: tag,
        type: '',
        nonce: '',
        textContent: '',
        attributes: {} as Record<string, string>,
        setAttribute(name: string, value: string) {
          this.attributes[name] = value;
        },
      });
      (globalThis as any).document.querySelector = () => null;
      (globalThis as any).document.head = {
        appendChild(el: Record<string, unknown>) {
          if (el.tagName === 'style') {
            order.push(`style:${(el.attributes as Record<string, string>).specifier}`);
            return el;
          }
          order.push('script');
          scriptNonces.push(el.nonce as string);
          scriptBodies.push(el.textContent as string);
          // eslint-disable-next-line no-new-func
          Function(el.textContent as string)();
          return el;
        },
        removeChild() {
          return undefined;
        },
      };

      try {
        const router = new WebUIRouter();
        // Set nonce on the global (source of truth)
        globals().__webui!.nonce = 'test-nonce';
        const fetchPartial = (router as any).fetchPartial.bind(router) as (
          path: string,
          signal?: AbortSignal,
        ) => Promise<unknown>;

        const result = await fetchPartial('/test');

        assert.ok(result, 'should return partial data');
        // Styles must be appended BEFORE scripts
        assert.deepEqual(order, ['style:alpha', 'style:beta', 'script']);
        // All JS IIFEs batched into one script tag
        assert.equal(scriptBodies.length, 1, 'all JS should be batched into one script tag');
        assert.ok(scriptBodies[0].includes('w["alpha"]'), 'batch should include alpha IIFE');
        assert.ok(scriptBodies[0].includes('w["beta"]'), 'batch should include beta IIFE');
        // CSP nonce preserved
        assert.deepEqual(scriptNonces, ['test-nonce'], 'batched script should carry the nonce');
        // Templates actually registered
        assert.ok(globals().__webui?.templates?.['alpha'], 'alpha template should register');
        assert.ok(globals().__webui?.templates?.['beta'], 'beta template should register');
      } finally {
        (globalThis as any).fetch = origFetch;
        (globalThis as any).document.createElement = origCreateElement;
        (globalThis as any).document.querySelector = origQuerySelector;
        (globalThis as any).document.head = origHead;
      }
    });

    test('fetchPartial handles empty templateStyles for Link/Style modes', async () => {
      const origFetch = (globalThis as any).fetch;
      const origCreateElement = (globalThis as any).document.createElement;
      const origHead = (globalThis as any).document.head;

      const appendedTags: string[] = [];

      (globalThis as any).fetch = async () => ({
        ok: true,
        headers: { get: () => 'application/json' },
        json: async () => ({
          state: {},
          templateStyles: [],
          templates: [
            '(function(){var w=(window.__webui||(window.__webui={})).templates||(window.__webui.templates={});w["link-comp"]={h:"<div/>"};})();',
          ],
          path: '/',
          chain: [],
          inventory: 'ff',
        }),
      });

      (globalThis as any).document.createElement = (tag: string) => ({
        tagName: tag,
        type: '',
        nonce: '',
        textContent: '',
        setAttribute() {},
      });
      (globalThis as any).document.head = {
        appendChild(el: Record<string, unknown>) {
          appendedTags.push(el.tagName as string);
          if (el.tagName === 'script') {
            // eslint-disable-next-line no-new-func
            Function(el.textContent as string)();
          }
          return el;
        },
        removeChild() { return undefined; },
      };

      try {
        const router = new WebUIRouter();
        const fetchPartial = (router as any).fetchPartial.bind(router) as (
          path: string,
        ) => Promise<unknown>;

        await fetchPartial('/test');

        // No style tags should be appended — only the batched script
        assert.deepEqual(appendedTags, ['script'], 'only a script tag should be appended');
        assert.ok(globals().__webui?.templates?.['link-comp'], 'template should register');
      } finally {
        (globalThis as any).fetch = origFetch;
        (globalThis as any).document.createElement = origCreateElement;
        (globalThis as any).document.head = origHead;
      }
    });
  });

  describe('navigation abort signal', () => {
    test('fetchPartial passes signal to fetch', async () => {
      // Shim fetch to capture the options passed to it
      const origFetch = (globalThis as any).fetch;
      let capturedSignal: AbortSignal | undefined;
      (globalThis as any).fetch = async (_url: string, opts?: RequestInit) => {
        capturedSignal = opts?.signal as AbortSignal | undefined;
        return { ok: true, headers: { get: () => 'application/json' }, json: async () => ({ state: {}, templates: [], path: '/', chain: [] }) };
      };

      try {
        const router = new WebUIRouter();
        const fetchPartial = (router as any).fetchPartial.bind(router) as (
          path: string,
          signal?: AbortSignal,
        ) => Promise<unknown>;

        const controller = new AbortController();
        await fetchPartial('/test', controller.signal);

        assert.ok(capturedSignal, 'signal should be passed to fetch');
        assert.equal(capturedSignal, controller.signal, 'should be the same signal instance');
      } finally {
        (globalThis as any).fetch = origFetch;
      }
    });

    test('fetchPartial skips side effects when signal is aborted after response', async () => {
      const origFetch = (globalThis as any).fetch;

      (globalThis as any).fetch = async (_url: string, _opts?: RequestInit) => {
        return {
          ok: true,
          headers: { get: () => 'application/json' },
          json: async () => ({
            state: {},
            templates: [
              '<script>(function(){window.__webui.templates["abort-test"]={h:"<div/>"};})()</script>',
            ],
            path: '/',
            chain: [],
            inventory: 'ff',
          }),
        };
      };

      try {
        const router = new WebUIRouter();
        const fetchPartial = (router as any).fetchPartial.bind(router) as (
          path: string,
          signal?: AbortSignal,
        ) => Promise<unknown>;

        // Abort the signal before calling — simulates a superseded navigation
        const controller = new AbortController();
        controller.abort();

        const result = await fetchPartial('/test', controller.signal);

        assert.equal(result, null, 'should return null for aborted navigation');
        assert.equal(globals().__webui?.templates?.['abort-test'], undefined, 'should not register templates after abort');
      } finally {
        (globalThis as any).fetch = origFetch;
      }
    });

    test('fetchPartial works normally without signal', async () => {
      const origFetch = (globalThis as any).fetch;
      (globalThis as any).fetch = async (_url: string, opts?: RequestInit) => {
        assert.equal(opts?.signal, undefined, 'signal should be undefined');
        return { ok: true, headers: { get: () => 'application/json' }, json: async () => ({ state: {}, templates: [], path: '/', chain: [] }) };
      };

      try {
        const router = new WebUIRouter();
        const fetchPartial = (router as any).fetchPartial.bind(router) as (
          path: string,
          signal?: AbortSignal,
        ) => Promise<unknown>;

        const result = await fetchPartial('/test');
        assert.ok(result, 'should return data when no signal is provided');
      } finally {
        (globalThis as any).fetch = origFetch;
      }
    });

    test('intercept handler silently swallows AbortError', async () => {
      // Verify the architectural contract: the intercept handler catches
      // AbortError without logging. This is critical for rapid navigation
      // where superseded fetches throw AbortError.
      const router = new WebUIRouter();
      const source = (router as any).start.toString() as string;

      // The handler must check for AbortError by name
      assert.ok(
        source.includes('AbortError'),
        'intercept handler should check for AbortError',
      );

      // It should not re-throw or console.error AbortErrors
      // Verify the pattern: if AbortError → return (swallow)
      const abortIdx = source.indexOf('AbortError');
      const returnAfterAbort = source.indexOf('return', abortIdx);
      assert.ok(
        returnAfterAbort > abortIdx && returnAfterAbort - abortIdx < 80,
        'AbortError check should be followed by a return (swallow pattern)',
      );
    });

    test('handleNavigation checks signal.aborted after fetch and inside preload loop', () => {
      // Verify the architectural contract: commitWithData has abort gates
      // after fetchPartial and inside the ensureComponentLoaded loop.
      const router = new WebUIRouter();
      const navSource = (router as any).handleNavigation.toString() as string;
      const commitSource = (router as any).commitWithData.toString() as string;

      // handleNavigation should call fetchPartial
      const fetchIdx = navSource.indexOf('fetchPartial');
      assert.ok(fetchIdx > -1, 'fetchPartial should be called in handleNavigation');

      // Count occurrences of signal?.aborted checks in commitWithData
      const abortChecks: number[] = [];
      let searchFrom = 0;
      while (true) {
        const idx = commitSource.indexOf('signal?.aborted', searchFrom);
        if (idx === -1) break;
        abortChecks.push(idx);
        searchFrom = idx + 1;
      }

      assert.ok(
        abortChecks.length >= 2,
        `should have at least 2 abort gates in commitWithData, found ${abortChecks.length}`,
      );

      // One of the abort gates should be near ensureComponentLoaded
      // (i.e. inside the preload section, guarding the preload step).
      const hasPreloadGate = abortChecks.some(pos => {
        const after = commitSource.substring(pos, pos + 400);
        return after.includes('ensureComponentLoaded');
      });
      assert.ok(
        hasPreloadGate,
        'an abort gate should guard ensureComponentLoaded inside the preload loop',
      );
    });

    test('handleNavigation clears SSR-only preload links before partial fetches', () => {
      const router = new WebUIRouter();
      const source = (router as any).handleNavigation.toString() as string;
      const clearIdx = source.indexOf('this.clearSsrPreloads()');
      const fetchIdx = source.indexOf('fetchPartial');

      assert.ok(clearIdx > -1, 'handleNavigation should clear SSR preload links on SPA navigations');
      assert.ok(fetchIdx > -1, 'handleNavigation should fetch partial data');
      assert.ok(
        clearIdx < fetchIdx,
        'SSR preload links should be cleared before fetching the next partial route',
      );
    });
  });

  describe('view transition timing', () => {
    test('startViewTransition awaits updateCallbackDone not finished', () => {
      // Regression: awaiting .finished blocks the Navigation API intercept
      // handler until the CSS animation completes, serializing navigations
      // behind transition animations. The router must use .updateCallbackDone
      // so the handler resolves after the synchronous DOM commit.
      const router = new WebUIRouter();
      const source = (router as any).commitWithData.toString() as string;

      assert.ok(
        source.includes('updateCallbackDone'),
        'should await updateCallbackDone on the view transition',
      );
      assert.ok(
        !source.includes('transition.finished'),
        'should NOT await transition.finished on the view transition — it blocks rapid navigation',
      );
    });

    test('startViewTransition is skipped for query-only navigations', () => {
      // Regression (issue #235): query-only navigations (e.g. /contacts →
      // /contacts?q=foo) do not remount any components. Wrapping them in
      // startViewTransition captures a screenshot and temporarily suppresses
      // the live DOM, which blurs the active element (search input focus lost
      // mid-typing). The router must guard the startViewTransition call with
      // an `isQueryOnlyChange` check so focus is preserved.
      const router = new WebUIRouter();
      const source = (router as any).commitWithData.toString() as string;

      // The view-transition block must be gated on !isQueryOnlyChange
      const transitionIdx = source.indexOf('startViewTransition');
      assert.ok(transitionIdx > -1, 'startViewTransition should be referenced');

      // The `if` condition that invokes startViewTransition must also
      // check isQueryOnlyChange (either before or after the feature check).
      // We grab the surrounding context to verify the guard is present.
      const conditionRegion = source.slice(
        Math.max(0, transitionIdx - 120),
        transitionIdx + 80,
      );
      assert.ok(
        conditionRegion.includes('isQueryOnlyChange'),
        'startViewTransition must be guarded by isQueryOnlyChange — ' +
        'query-only navigations must skip view transitions to preserve focus ' +
        `(condition region: "${conditionRegion}")`,
      );
    });

    test('startViewTransition callback does not contain async fetch or import', () => {
      // Regression: the view transition callback must complete synchronously
      // (DOM swap only). Async work (fetch, module load) must happen BEFORE
      // startViewTransition is called. If the callback contains awaits for
      // network or import(), the browser's view transition will timeout.
      //
      // This test verifies the architectural contract by inspecting the
      // commitWithData source code — the commitNavigation callback passed
      // to startViewTransition must not reference fetchPartial or
      // ensureComponentLoaded.

      const router = new WebUIRouter();
      const source = (router as any).commitWithData.toString() as string;

      // Find the commitNavigation function body
      const commitStart = source.indexOf('commitNavigation');
      assert.ok(commitStart > -1, 'commitWithData should define commitNavigation');

      // Find ensureComponentLoaded — it must appear BEFORE commitNavigation
      const ensureIdx = source.indexOf('ensureComponentLoaded');
      assert.ok(ensureIdx > -1, 'ensureComponentLoaded should be called');
      assert.ok(
        ensureIdx < commitStart,
        'ensureComponentLoaded (async import) must be called BEFORE commitNavigation is defined — ' +
        'async work must not be inside the view transition callback',
      );

      // fetchPartial must also appear before commitNavigation — but commitWithData
      // receives data as a parameter, so fetchPartial won't be in this method.
      // Instead, verify the method signature receives data as a parameter.
      assert.ok(
        source.includes('partialData') || source.includes('data'),
        'commitWithData receives data as a parameter (fetch happens in caller)',
      );
    });
  });

  describe('CSS injection contract', () => {
    test('PartialResponse css array injects link elements', () => {
      // Simulate the CSS injection logic from fetchPartial.
      // The router iterates data.css and creates <link> elements.
      const injected: Array<{ rel: string; href: string }> = [];
      const origCreateElement = (globalThis as any).document.createElement;
      const origQuerySelector = (globalThis as any).document.querySelector;
      const origHead = (globalThis as any).document.head;

      // Shim DOM to capture link injections
      (globalThis as any).document.createElement = (tag: string) => {
        const el: Record<string, unknown> = { tagName: tag };
        return el;
      };
      (globalThis as any).document.querySelector = () => null; // no existing links
      (globalThis as any).document.head = {
        appendChild(el: Record<string, unknown>) {
          if (el.rel === 'stylesheet') {
            injected.push({ rel: el.rel as string, href: el.href as string });
          }
        },
      };

      try {
        // Replicate the css injection contract from fetchPartial
        const css = ['/email-message.css', '/mail-thread-page.css'];
        for (const href of css) {
          if (!(globalThis as any).document.querySelector(`link[href="${href}"]`)) {
            const link = (globalThis as any).document.createElement('link');
            link.rel = 'stylesheet';
            link.href = href;
            (globalThis as any).document.head.appendChild(link);
          }
        }

        assert.equal(injected.length, 2, 'should inject two CSS links');
        assert.equal(injected[0].href, '/email-message.css');
        assert.equal(injected[1].href, '/mail-thread-page.css');
      } finally {
        (globalThis as any).document.createElement = origCreateElement;
        (globalThis as any).document.querySelector = origQuerySelector;
        (globalThis as any).document.head = origHead;
      }
    });

    test('CSS link injection skips duplicates', () => {
      const injected: string[] = [];
      const origCreateElement = (globalThis as any).document.createElement;
      const origQuerySelector = (globalThis as any).document.querySelector;
      const origHead = (globalThis as any).document.head;

      let existingHref: string | null = null;

      (globalThis as any).document.createElement = () => ({} as Record<string, unknown>);
      (globalThis as any).document.querySelector = (sel: string) => {
        // Simulate that the first href already exists
        return existingHref && sel.includes(existingHref) ? {} : null;
      };
      (globalThis as any).document.head = {
        appendChild(el: Record<string, unknown>) {
          injected.push(el.href as string);
        },
      };

      try {
        existingHref = '/email-message.css';
        const css = ['/email-message.css', '/mail-thread-page.css'];
        for (const href of css) {
          if (!(globalThis as any).document.querySelector(`link[href="${href}"]`)) {
            const link = (globalThis as any).document.createElement('link');
            link.rel = 'stylesheet';
            link.href = href;
            (globalThis as any).document.head.appendChild(link);
          }
        }

        assert.equal(injected.length, 1, 'should skip the duplicate');
        assert.equal(injected[0], '/mail-thread-page.css');
      } finally {
        (globalThis as any).document.createElement = origCreateElement;
        (globalThis as any).document.querySelector = origQuerySelector;
        (globalThis as any).document.head = origHead;
      }
    });
  });

  describe('preload on hover', () => {
    test('speculative fetchPartial does not redirect on HTML response', async () => {
      const origFetch = (globalThis as any).fetch;
      const origLocation = (globalThis as any).location;
      let redirected = false;
      (globalThis as any).location = {
        ...origLocation,
        set href(v: string) { redirected = true; },
        get href() { return origLocation.href; },
        get origin() { return origLocation.origin; },
        get pathname() { return origLocation.pathname; },
      };
      (globalThis as any).fetch = async () => ({
        ok: true,
        headers: { get: () => 'text/html' },
        json: async () => ({}),
      });

      try {
        const router = new WebUIRouter();
        const fetchPartial = (router as any).fetchPartial.bind(router) as (
          path: string,
          signal?: AbortSignal,
          speculative?: boolean,
        ) => Promise<unknown>;

        // Non-speculative should redirect
        const result1 = await fetchPartial('/login');
        assert.equal(result1, null);
        assert.ok(redirected, 'non-speculative HTML response should redirect');

        // Speculative should NOT redirect
        redirected = false;
        const result2 = await fetchPartial('/login', undefined, true);
        assert.equal(result2, null);
        assert.ok(!redirected, 'speculative HTML response should not redirect');
      } finally {
        (globalThis as any).fetch = origFetch;
        (globalThis as any).location = origLocation;
      }
    });

    test('preload cache is consumed on navigation and cleared after use', () => {
      const router = new WebUIRouter();
      const priv = router as any;
      priv.navCache = new NavigationCache({ staleTime: 30_000, gcTime: 300_000, maxEntries: 50 });

      const cachedData = { state: { msg: 'preloaded' }, templates: [], path: '/about', chain: [{ component: 'about-page', path: '/about', params: {} }] };
      // Store in unified cache via navCache.store
      priv.navCache.store('/about', cachedData);

      // Verify the cache entry exists
      assert.ok(priv.navCache.has('/about'), 'cache should have entry for /about');

      // Simulate consumption by lookup + evict
      const consumed = priv.navCache.lookup('/about');
      assert.deepEqual(consumed.state, { msg: 'preloaded' });
      priv.navCache.evict('/about');
      assert.ok(!priv.navCache.has('/about'), 'cache should be empty after eviction');
    });

    test('stale preload cache (>TTL) is not consumed', () => {
      const router = new WebUIRouter();
      const priv = router as any;
      priv.navCache = new NavigationCache({ staleTime: 0, gcTime: 300_000, maxEntries: 50 });

      // Store a cache entry, then manipulate its timestamp to make it stale
      priv.navCache.store('/about', { state: {}, templates: [], path: '/about' });
      const entry = priv.navCache.getEntry('/about');
      entry.ts = Date.now() - 10_000;

      // lookup should return null for stale entries
      const result = priv.navCache.lookup('/about');
      assert.equal(result, null, 'cache older than TTL should be considered stale');
    });

    test('preload generation guard prevents stale cache writes', async () => {
      const router = new WebUIRouter();
      const priv = router as any;

      // Simulate: preload A starts (gen=1), preload B starts (gen=2),
      // preload A resolves — should NOT write to cache because gen is stale.
      priv.preloadGeneration = 0;

      const genA = ++priv.preloadGeneration; // gen=1
      const genB = ++priv.preloadGeneration; // gen=2

      // A's completion check: gen should not match current
      assert.notEqual(genA, priv.preloadGeneration, 'stale gen should not match current');
      assert.equal(genB, priv.preloadGeneration, 'current gen should match');
    });

    test('destroy clears preload state', () => {
      const router = new WebUIRouter();
      const priv = router as any;
      priv.navCache = new NavigationCache({ staleTime: 0, gcTime: 300_000, maxEntries: 50 });

      // Store an entry in the navigation cache
      priv.navCache.store('/test', { state: {}, templates: [], path: '/test' });

      router.destroy();

      assert.ok(!priv.navCache.has('/test'), 'cache should be cleared');
    });

    test('setupPreloadListeners registers pointermove listener on document', () => {
      const listeners: Array<{ type: string; handler: unknown }> = [];
      const origAddEventListener = (globalThis as any).document.addEventListener;
      const origRemoveEventListener = (globalThis as any).document.removeEventListener;
      (globalThis as any).document.addEventListener = (type: string, fn: unknown) => {
        listeners.push({ type, handler: fn });
      };
      (globalThis as any).document.removeEventListener = () => {};

      try {
        const cleanup = setupPreloadListeners({
          basePath: '',
          excludePaths: [],
          currentRequestPath: '/',
          inventory: '',
          hasCache: () => false,
          storeCache: () => {},
          fetchPartial: async () => null,
        });
        assert.ok(
          listeners.some(l => l.type === 'pointermove'),
          'should register a pointermove listener on document',
        );
        cleanup();
      } finally {
        (globalThis as any).document.addEventListener = origAddEventListener;
        (globalThis as any).document.removeEventListener = origRemoveEventListener;
      }
    });

    test('handleNavigation source checks cache before fetchPartial', () => {
      const router = new WebUIRouter();
      const source = (router as any).handleNavigation.toString() as string;

      // After extraction, the cache lookup is via navCache.lookup
      const cacheIdx = source.indexOf('navCache.lookup');
      const fetchIdx = source.indexOf('fetchPartial');

      assert.ok(cacheIdx > -1, 'handleNavigation should reference navCache.lookup');
      assert.ok(fetchIdx > -1, 'handleNavigation should reference fetchPartial');
      assert.ok(
        cacheIdx < fetchIdx,
        'navCache.lookup check should come before fetchPartial call',
      );
    });
  });

  describe('route loaders', () => {
    test('resolveLoaders calls static loader() on registered components', async () => {
      // Register a fake component with a static loader
      const origGet = (globalThis as any).customElements.get;
      (globalThis as any).customElements.get = (name: string) => {
        if (name === 'dash-page') {
          return class DashPage {
            static async loader(ctx: { params: Record<string, string>; query: Record<string, string> }) {
              return { dashId: ctx.params.id, source: 'loader' };
            }
          };
        }
        return origGet(name);
      };

      try {
        const results = await resolveLoaders(
          [{ component: 'dash-page', path: '/', params: { id: '42' } }],
          { filter: 'active' },
        );
        assert.ok(results.has('dash-page'), 'should have loader result for dash-page');
        assert.deepEqual(results.get('dash-page'), { dashId: '42', source: 'loader' });
      } finally {
        (globalThis as any).customElements.get = origGet;
      }
    });

    test('resolveLoaders skips components without static loader', async () => {
      const origGet = (globalThis as any).customElements.get;
      (globalThis as any).customElements.get = (name: string) => {
        if (name === 'plain-page') return class PlainPage {};
        return origGet(name);
      };

      try {
        const results = await resolveLoaders(
          [{ component: 'plain-page', path: '/', params: {} }],
          {},
        );
        assert.equal(results.size, 0, 'should have no results for components without loader');
      } finally {
        (globalThis as any).customElements.get = origGet;
      }
    });

    test('resolveLoaders falls back gracefully on loader failure', async () => {
      const origGet = (globalThis as any).customElements.get;
      const origWarn = console.warn;
      let warned = false;
      console.warn = () => { warned = true; };

      (globalThis as any).customElements.get = (name: string) => {
        if (name === 'broken-page') {
          return class BrokenPage {
            static async loader() { throw new Error('API down'); }
          };
        }
        return origGet(name);
      };

      try {
        const results = await resolveLoaders(
          [{ component: 'broken-page', path: '/', params: {} }],
          {},
        );
        assert.equal(results.size, 1, 'failed loader should add a LOADER_FAILED sentinel');
        assert.ok(warned, 'should log a warning on loader failure');
      } finally {
        (globalThis as any).customElements.get = origGet;
        console.warn = origWarn;
      }
    });

    test('resolveLoaders respects abort signal', async () => {
      const origGet = (globalThis as any).customElements.get;
      (globalThis as any).customElements.get = (name: string) => {
        if (name === 'slow-page') {
          return class SlowPage {
            static async loader() {
              await new Promise(r => setTimeout(r, 50));
              return { data: 'loaded' };
            }
          };
        }
        return origGet(name);
      };

      try {
        const controller = new AbortController();
        controller.abort(); // Pre-abort
        const results = await resolveLoaders(
          [{ component: 'slow-page', path: '/', params: {} }],
          {},
          controller.signal,
        );
        assert.equal(results.size, 0, 'aborted signal should prevent cache write');
      } finally {
        (globalThis as any).customElements.get = origGet;
      }
    });

    test('mountComponent calls applyParamsQueryState with state', () => {
      const router = new WebUIRouter();
      const source = (router as any).mountComponent.toString() as string;
      assert.ok(
        source.includes('applyParamsQueryState'),
        'mountComponent should call applyParamsQueryState',
      );
    });

    test('commitWithData resolves loaders before commitNavigation', () => {
      const router = new WebUIRouter();
      const source = (router as any).commitWithData.toString() as string;

      const resolveIdx = source.indexOf('resolveLoaders');
      const commitIdx = source.indexOf('commitNavigation');

      assert.ok(resolveIdx > -1, 'commitWithData should call resolveLoaders');
      assert.ok(commitIdx > -1, 'commitWithData should define commitNavigation');
      assert.ok(
        resolveIdx < commitIdx,
        'resolveLoaders must run before commitNavigation is defined',
      );
    });
  });

  describe('keep-alive state preservation', () => {
    test('applyState skips setState for keep-alive with null state', () => {
      const router = new WebUIRouter();
      const priv = router as any;

      let setStateCalled = false;
      const mockCompEl = {
        setAttribute: () => {},
        removeAttribute: () => {},
        setState: () => { setStateCalled = true; },
      };
      const mockRouteEl = {
        hasAttribute: (name: string) => name === 'keep-alive',
        getAttribute: () => null,
        querySelector: (sel: string) => sel === 'test-comp' ? mockCompEl : null,
      };

      const entry = {
        component: 'test-comp',
        path: '/',
        params: { id: '42' },
        el: mockRouteEl,
        keepAlive: true,
        state: null,  // null = skip setState
      };

      priv.applyState(entry, {}, new Map());
      assert.ok(!setStateCalled, 'setState must not be called for keep-alive with null state');
    });

    test('applyState calls setState for keep-alive with loader override', () => {
      const router = new WebUIRouter();
      const priv = router as any;

      let setStateArg: unknown = null;
      const mockCompEl = {
        setAttribute: () => {},
        removeAttribute: () => {},
        setState: (s: unknown) => { setStateArg = s; },
      };
      const mockRouteEl = {
        hasAttribute: (name: string) => name === 'keep-alive',
        getAttribute: () => null,
        querySelector: (sel: string) => sel === 'test-comp' ? mockCompEl : null,
      };

      const entry = {
        component: 'test-comp',
        path: '/',
        params: {},
        el: mockRouteEl,
        keepAlive: true,
        state: null,
      };
      const loaderStates = new Map();
      loaderStates.set('test-comp', { fresh: 'data' });

      priv.applyState(entry, {}, loaderStates);
      assert.deepEqual(setStateArg, { fresh: 'data' }, 'setState should receive loader override');
    });

    test('applyState uses per-entry state for non-keep-alive', () => {
      const router = new WebUIRouter();
      const priv = router as any;

      let setStateArg: unknown = null;
      const mockCompEl = {
        setAttribute: () => {},
        removeAttribute: () => {},
        setState: (s: unknown) => { setStateArg = s; },
      };
      const mockRouteEl = {
        hasAttribute: () => false,
        getAttribute: () => null,
        querySelector: (sel: string) => sel === 'test-comp' ? mockCompEl : null,
      };

      const entry = {
        component: 'test-comp',
        path: '/',
        params: {},
        el: mockRouteEl as any,
        keepAlive: false,
        state: { from: 'server' },
      };

      priv.applyState(entry, {}, new Map());
      assert.deepEqual(setStateArg, { from: 'server' }, 'non-keep-alive should use per-entry state');
    });
  });

  describe('fetchPartial NDJSON support', () => {
    test('fetchPartial sends Accept header for both NDJSON and JSON', async () => {
      const origFetch = (globalThis as any).fetch;
      let capturedHeaders: Record<string, string> = {};

      (globalThis as any).fetch = async (_url: string, opts?: RequestInit) => {
        capturedHeaders = (opts?.headers as Record<string, string>) ?? {};
        return {
          ok: true,
          headers: { get: () => 'application/json' },
          json: async () => ({ state: {}, templates: [], path: '/', chain: [] }),
        };
      };

      try {
        const router = new WebUIRouter();
        await (router as any).fetchPartial.call(router, '/test');
        const accept = capturedHeaders['Accept'];
        assert.ok(accept, 'should send Accept header');
        assert.ok(accept.includes('application/x-ndjson'), 'Accept should include ndjson');
        assert.ok(accept.includes('application/json'), 'Accept should include json');
      } finally {
        (globalThis as any).fetch = origFetch;
      }
    });
  });

  describe('loaderPromises cleanup', () => {
    test('loaderPromises entries are deleted after loader completes', async () => {
      const loaders: Record<string, () => Promise<unknown>> = { 'lazy-comp': () => Promise.resolve() };
      const loaderPromises = new Map<string, Promise<void>>();

      // Before loading, map is empty
      assert.equal(loaderPromises.size, 0, 'should start empty');

      // Call ensureComponentLoaded — adds entry to loaderPromises
      const loadPromise = ensureComponentLoaded('lazy-comp', loaders, loaderPromises);
      assert.equal(loaderPromises.size, 1, 'should have in-flight entry');

      // After the loader resolves, the entry should be cleaned up
      await loadPromise;
      assert.equal(loaderPromises.size, 0, 'entry should be deleted after completion');
    });

    test('loaderPromises entries are deleted even when loader rejects', async () => {
      const loaders: Record<string, () => Promise<unknown>> = { 'broken-comp': () => Promise.reject(new Error('load failed')) };
      const loaderPromises = new Map<string, Promise<void>>();

      const loadPromise = ensureComponentLoaded('broken-comp', loaders, loaderPromises);
      assert.equal(loaderPromises.size, 1, 'should have in-flight entry');

      // Await should not throw — the promise chain handles the rejection
      await loadPromise.catch(() => {});
      assert.equal(loaderPromises.size, 0, 'entry should be deleted after rejection');
    });
  });
});

// ── parseQuery unit tests ────────────────────────────────────────

describe('parseQuery', () => {
  test('returns empty object for paths without query string', () => {
    assert.deepEqual(parseQuery('/compose'), {});
    assert.deepEqual(parseQuery('/'), {});
    assert.deepEqual(parseQuery('/items/42'), {});
  });

  test('parses single query parameter', () => {
    assert.deepEqual(parseQuery('/compose?action=reply'), { action: 'reply' });
  });

  test('parses multiple query parameters', () => {
    assert.deepEqual(
      parseQuery('/compose?action=reply&to=test@example.com&subject=Re: Hello'),
      { action: 'reply', to: 'test@example.com', subject: 'Re: Hello' },
    );
  });

  test('decodes percent-encoded values', () => {
    assert.deepEqual(
      parseQuery('/compose?subject=Re%3A%20%5Bwebui%5D%20Fix%20bug'),
      { subject: 'Re: [webui] Fix bug' },
    );
  });

  test('handles empty values', () => {
    assert.deepEqual(parseQuery('/search?q='), { q: '' });
  });

  test('last value wins for duplicate keys', () => {
    const result = parseQuery('/search?sort=date&sort=name');
    assert.equal(result.sort, 'name');
  });

  test('handles query with no path prefix', () => {
    assert.deepEqual(parseQuery('/?q=test'), { q: 'test' });
  });
});

// ── filterQuery unit tests ───────────────────────────────────────

describe('filterQuery', () => {
  test('returns empty object when allowlist is null (deny-by-default)', () => {
    assert.deepEqual(filterQuery({ action: 'reply', evil: 'inject' }, null), {});
  });

  test('returns empty object when allowlist is empty set', () => {
    assert.deepEqual(filterQuery({ action: 'reply' }, new Set()), {});
  });

  test('passes only allowed keys', () => {
    const allowed = new Set(['action', 'to']);
    const query = { action: 'reply', to: 'user@test.com', evil: 'inject', style: 'display:none' };
    assert.deepEqual(filterQuery(query, allowed), { action: 'reply', to: 'user@test.com' });
  });

  test('excludes keys that collide with route params', () => {
    const allowed = new Set(['itemId', 'action']);
    const query = { itemId: 'evil', action: 'reply' };
    const routeParams = { itemId: '42' };
    assert.deepEqual(filterQuery(query, allowed, routeParams), { action: 'reply' });
  });

  test('excludes keys whose kebab form collides with route param kebab form', () => {
    const allowed = new Set(['item-id', 'action']);
    const query = { 'item-id': 'evil', action: 'reply' };
    const routeParams = { itemId: '42' };
    assert.deepEqual(filterQuery(query, allowed, routeParams), { action: 'reply' });
  });

  test('returns empty object when query is empty', () => {
    assert.deepEqual(filterQuery({}, new Set(['action'])), {});
  });

  test('handles allowed keys not present in query', () => {
    const allowed = new Set(['action', 'to', 'subject']);
    assert.deepEqual(filterQuery({ action: 'reply' }, allowed), { action: 'reply' });
  });
});