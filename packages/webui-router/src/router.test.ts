// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Shim must be imported before any router code — sets up browser globals.
import './browser-shim.js';

import { strict as assert } from 'node:assert';
import { describe, test, beforeEach, afterEach } from 'node:test';
import { encodeInventoryHex } from './inventory.js';
import { WebUIRouter } from './router.js';

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
  __webui_templates?: Record<string, unknown>;
}

function globals(): TemplateRegistry {
  return globalThis as unknown as TemplateRegistry;
}

/** Build a 32-byte inventory with specific bits set via component names. */
function inventoryWith(...names: string[]): string {
  function bitPosition(name: string): number {
    let hash = 0x811c9dc5;
    for (let i = 0; i < name.length; i++) {
      hash ^= name.charCodeAt(i);
      hash = Math.imul(hash, 0x01000193) >>> 0;
    }
    return hash % 256;
  }
  const inv = new Uint8Array(32);
  for (const n of names) {
    const bit = bitPosition(n);
    inv[bit >> 3] |= 1 << (bit & 7);
  }
  return encodeInventoryHex(inv);
}

/** Check if a bit is set for a component name in an inventory hex string. */
function hasBit(hex: string, name: string): boolean {
  function bitPosition(n: string): number {
    let hash = 0x811c9dc5;
    for (let i = 0; i < n.length; i++) {
      hash ^= n.charCodeAt(i);
      hash = Math.imul(hash, 0x01000193) >>> 0;
    }
    return hash % 256;
  }
  const bytes = new Uint8Array(hex.length >> 1);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.substring(i * 2, i * 2 + 2), 16);
  }
  const bit = bitPosition(name);
  const byteIdx = bit >> 3;
  return byteIdx < bytes.length && (bytes[byteIdx] & (1 << (bit & 7))) !== 0;
}

describe('WebUIRouter', () => {
  let savedTemplates: Record<string, unknown> | undefined;

  beforeEach(() => {
    savedTemplates = globals().__webui_templates;
    globals().__webui_templates = {};
  });

  afterEach(() => {
    if (savedTemplates === undefined) {
      delete globals().__webui_templates;
    } else {
      globals().__webui_templates = savedTemplates;
    }
  });

  describe('releaseTemplates', () => {
    test('releases specific templates and clears inventory bits', () => {
      const router = new WebUIRouter();
      const registry = globals().__webui_templates!;
      registry['page-a'] = { h: '<div>A</div>' };
      registry['page-b'] = { h: '<div>B</div>' };
      registry['page-c'] = { h: '<div>C</div>' };

      internals(router).inventory = inventoryWith('page-a', 'page-b', 'page-c');

      router.releaseTemplates(['page-a', 'page-c']);

      assert.equal(registry['page-a'], undefined, 'page-a should be deleted');
      assert.ok(registry['page-b'], 'page-b should be retained');
      assert.equal(registry['page-c'], undefined, 'page-c should be deleted');

      const inv = internals(router).inventory;
      assert.equal(hasBit(inv, 'page-a'), false, 'inventory bit for page-a should be cleared');
      assert.equal(hasBit(inv, 'page-b'), true, 'inventory bit for page-b should remain');
      assert.equal(hasBit(inv, 'page-c'), false, 'inventory bit for page-c should be cleared');
    });

    test('releases all non-active templates when called with no args', () => {
      const router = new WebUIRouter();
      const registry = globals().__webui_templates!;
      registry['shell'] = { h: '<div>Shell</div>' };
      registry['page-x'] = { h: '<div>X</div>' };
      registry['page-y'] = { h: '<div>Y</div>' };

      const ri = internals(router);
      ri.inventory = inventoryWith('shell', 'page-x', 'page-y');
      ri.activeChain = [{ component: 'shell', path: '/', params: {} }];

      router.releaseTemplates();

      assert.ok(registry['shell'], 'active component should be retained');
      assert.equal(registry['page-x'], undefined, 'inactive page-x should be released');
      assert.equal(registry['page-y'], undefined, 'inactive page-y should be released');
    });

    test('skips active components even when explicitly requested', () => {
      const router = new WebUIRouter();
      const registry = globals().__webui_templates!;
      registry['active-comp'] = { h: '<div>active</div>' };
      registry['inactive-comp'] = { h: '<div>inactive</div>' };

      const ri = internals(router);
      ri.inventory = inventoryWith('active-comp', 'inactive-comp');
      ri.activeChain = [{ component: 'active-comp', path: '/', params: {} }];

      router.releaseTemplates(['active-comp', 'inactive-comp']);

      assert.ok(registry['active-comp'], 'active component must not be released');
      assert.equal(registry['inactive-comp'], undefined, 'inactive component should be released');
      assert.equal(hasBit(ri.inventory, 'active-comp'), true, 'active bit must remain set');
    });

    test('is a no-op when no templates are registered', () => {
      const router = new WebUIRouter();
      globals().__webui_templates = undefined;

      // Should not throw
      router.releaseTemplates();
      router.releaseTemplates(['anything']);
    });

    test('is a no-op when all requested tags are active', () => {
      const router = new WebUIRouter();
      const registry = globals().__webui_templates!;
      registry['only-one'] = { h: '<div/>' };

      const origInventory = inventoryWith('only-one');
      const ri = internals(router);
      ri.inventory = origInventory;
      ri.activeChain = [{ component: 'only-one', path: '/', params: {} }];

      router.releaseTemplates(['only-one']);

      assert.ok(registry['only-one'], 'should not release');
      assert.equal(ri.inventory, origInventory, 'inventory should be unchanged');
    });

    test('handles nested active chain correctly', () => {
      const router = new WebUIRouter();
      const registry = globals().__webui_templates!;
      registry['app-shell'] = { h: '<div>shell</div>' };
      registry['section-page'] = { h: '<div>section</div>' };
      registry['topic-page'] = { h: '<div>topic</div>' };
      registry['lesson-page'] = { h: '<div>lesson</div>' };

      const ri = internals(router);
      ri.inventory = inventoryWith('app-shell', 'section-page', 'topic-page', 'lesson-page');
      ri.activeChain = [
        { component: 'app-shell', path: '/', params: {} },
        { component: 'section-page', path: 'sections/:id', params: { id: '1' } },
        { component: 'topic-page', path: 'topics/:tid', params: { tid: 'react' } },
      ];

      router.releaseTemplates();

      assert.ok(registry['app-shell'], 'active shell retained');
      assert.ok(registry['section-page'], 'active section retained');
      assert.ok(registry['topic-page'], 'active topic retained');
      assert.equal(registry['lesson-page'], undefined, 'inactive lesson released');
    });
  });

  describe('template execution in fetchPartial', () => {
    test('Function-based execution registers templates without DOM', () => {
      // Simulate what fetchPartial does with the template script string
      const tmpl =
        '<script>(function(){var w=window.__webui_templates||(window.__webui_templates={});' +
        "w['test-comp']={h:\"<div>hello</div>\"};" +
        '})();</script>';

      const start = tmpl.indexOf('>') + 1;
      const end = tmpl.lastIndexOf('<');
      // eslint-disable-next-line no-new-func
      const run = Function(tmpl.substring(start, end));
      run();

      const registry = globals().__webui_templates!;
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
        json: async () => ({
          state: {},
          templateStyles: [
            '<style type="module" specifier="alpha">.alpha{color:red}</style>',
            '<style type="module" specifier="beta">.beta{color:blue}</style>',
          ],
          templates: [
            '(function(){var w=window.__webui_templates||(window.__webui_templates={});w["alpha"]={h:"<div>a</div>"};})();',
            '(function(){var w=window.__webui_templates||(window.__webui_templates={});w["beta"]={h:"<div>b</div>"};})();',
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
        (router as any).nonce = 'test-nonce';
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
        assert.ok(globals().__webui_templates?.['alpha'], 'alpha template should register');
        assert.ok(globals().__webui_templates?.['beta'], 'beta template should register');
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
        json: async () => ({
          state: {},
          templateStyles: [],
          templates: [
            '(function(){var w=window.__webui_templates||(window.__webui_templates={});w["link-comp"]={h:"<div/>"};})();',
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
        assert.ok(globals().__webui_templates?.['link-comp'], 'template should register');
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
        return { ok: true, json: async () => ({ state: {}, templates: [], path: '/', chain: [] }) };
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
      const origTemplates = globals().__webui_templates;
      globals().__webui_templates = {};

      (globalThis as any).fetch = async (_url: string, _opts?: RequestInit) => {
        return {
          ok: true,
          json: async () => ({
            state: {},
            templates: [
              '<script>(function(){window.__webui_templates["abort-test"]={h:"<div/>"};})()</script>',
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
        assert.equal(globals().__webui_templates?.['abort-test'], undefined, 'should not register templates after abort');
      } finally {
        (globalThis as any).fetch = origFetch;
        globals().__webui_templates = origTemplates;
      }
    });

    test('fetchPartial works normally without signal', async () => {
      const origFetch = (globalThis as any).fetch;
      (globalThis as any).fetch = async (_url: string, opts?: RequestInit) => {
        assert.equal(opts?.signal, undefined, 'signal should be undefined');
        return { ok: true, json: async () => ({ state: {}, templates: [], path: '/', chain: [] }) };
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
      // Verify the architectural contract: handleNavigation has abort gates
      // after fetchPartial and inside the ensureComponentLoaded loop.
      const router = new WebUIRouter();
      const source = (router as any).handleNavigation.toString() as string;

      // Count occurrences of signal?.aborted checks
      const abortChecks: number[] = [];
      let searchFrom = 0;
      while (true) {
        const idx = source.indexOf('signal?.aborted', searchFrom);
        if (idx === -1) break;
        abortChecks.push(idx);
        searchFrom = idx + 1;
      }

      assert.ok(
        abortChecks.length >= 2,
        `should have at least 2 abort gates, found ${abortChecks.length}`,
      );

      // First gate should be after fetchPartial
      const fetchIdx = source.indexOf('fetchPartial');
      assert.ok(fetchIdx > -1, 'fetchPartial should be called');
      assert.ok(
        abortChecks[0] > fetchIdx,
        'first abort gate should be after fetchPartial call',
      );

      // One of the abort gates should be immediately followed by ensureComponentLoaded
      // (i.e. inside the preload loop, guarding each iteration).
      const hasPreloadGate = abortChecks.some(pos => {
        const after = source.substring(pos, pos + 200);
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
      const source = (router as any).handleNavigation.toString() as string;

      assert.ok(
        source.includes('updateCallbackDone'),
        'should await updateCallbackDone on the view transition',
      );
      assert.ok(
        !source.includes('transition.finished'),
        'should NOT await transition.finished on the view transition — it blocks rapid navigation',
      );
    });

    test('startViewTransition callback does not contain async fetch or import', () => {
      // Regression: the view transition callback must complete synchronously
      // (DOM swap only). Async work (fetch, module load) must happen BEFORE
      // startViewTransition is called. If the callback contains awaits for
      // network or import(), the browser's view transition will timeout.
      //
      // This test verifies the architectural contract by inspecting the
      // handleNavigation source code — the commitNavigation callback passed
      // to startViewTransition must not reference fetchPartial or
      // ensureComponentLoaded.

      const router = new WebUIRouter();
      const source = (router as any).handleNavigation.toString() as string;

      // Find the commitNavigation function body
      const commitStart = source.indexOf('commitNavigation');
      assert.ok(commitStart > -1, 'handleNavigation should define commitNavigation');

      // Find ensureComponentLoaded — it must appear BEFORE commitNavigation
      const ensureIdx = source.indexOf('ensureComponentLoaded');
      assert.ok(ensureIdx > -1, 'ensureComponentLoaded should be called');
      assert.ok(
        ensureIdx < commitStart,
        'ensureComponentLoaded (async import) must be called BEFORE commitNavigation is defined — ' +
        'async work must not be inside the view transition callback',
      );

      // fetchPartial must also appear before commitNavigation
      const fetchIdx = source.indexOf('fetchPartial');
      assert.ok(fetchIdx > -1, 'fetchPartial should be called');
      assert.ok(
        fetchIdx < commitStart,
        'fetchPartial (async network) must be called BEFORE commitNavigation — ' +
        'async work must not be inside the view transition callback',
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
});
