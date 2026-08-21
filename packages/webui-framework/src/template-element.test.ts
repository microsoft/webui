// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import type { TemplateMeta } from './template.js';

/**
 * `TemplateElement extends HTMLElement` at module scope, so `HTMLElement`
 * must exist globally before the module is evaluated — same mocking pattern
 * as `static-host.test.ts` and `streaming.test.ts`.
 */
Object.defineProperty(globalThis, 'HTMLElement', {
  value: class HTMLElement {
    tagName = '';
    isConnected = false;
    childNodes: unknown[] = [];
    shadowRoot = null;
    _attrs: Record<string, string> = {};

    hasAttribute(name: string): boolean {
      return Object.prototype.hasOwnProperty.call(this._attrs, name);
    }

    getAttribute(name: string): string | null {
      return Object.prototype.hasOwnProperty.call(this._attrs, name) ? this._attrs[name] : null;
    }

    setAttribute(name: string, value: string): void {
      this._attrs[name] = String(value);
    }

    removeAttribute(name: string): void {
      delete this._attrs[name];
    }
  },
  configurable: true,
});

Object.defineProperty(globalThis, 'document', {
  value: {
    readyState: 'loading',
    getElementById() {
      return null;
    },
    querySelector(selector: string) {
      // Matches the real streaming meta tag detection query in
      // streaming-mode.ts — asserted so this test breaks loudly if that
      // selector ever changes instead of silently detecting non-streaming.
      assert.equal(selector, 'meta[name="webui-streaming"][content="1"]');
      return { getAttribute: () => '1' };
    },
  },
  configurable: true,
});

const dispatchedEvents: string[] = [];
Object.defineProperty(globalThis, 'window', {
  value: {
    __webui: { templates: {} },
    dispatchEvent(event: Event): boolean {
      dispatchedEvents.push(event.type);
      return true;
    },
  },
  configurable: true,
});

Object.defineProperty(globalThis, 'customElements', {
  value: {
    get() {
      return undefined;
    },
  },
  configurable: true,
});

const { TemplateElement } = await import('./template-element.js');
const { registerTemplateData } = await import('./template.js');
const {
  ACTIVATION_ACTIVATED,
  ACTIVATION_ANCESTOR_BARRIER,
  ACTIVATION_MISSING_TEMPLATE,
  ACTIVATION_STATIC_HOST_OPT_OUT,
  resetStreamingModeForTests,
} = await import('./streaming-mode.js');
const {
  beginStreamingGate,
  markBoundaryPending,
  markBoundaryCommitted,
  __getLifecycleStateForTests,
  __resetLifecycleForTests,
} = await import('./lifecycle.js');

/** The activation hook the streaming coordinator invokes on a committed boundary. */
const STREAMING_BOUNDARY_ACTIVATE = Symbol.for('microsoft.webui.boundaryActivate');
const STREAMING_BOUNDARY_ABANDON = Symbol.for('microsoft.webui.boundaryAbandon');
const PENDING_ROOT_CONNECTED = Symbol.for('microsoft.webui.pendingRootConnected');

/** Register template metadata for a tag exactly like `registerTemplateData()`. */
function registerTemplate(tag: string): void {
  (window as unknown as { __webui: { templates: Record<string, unknown> } }).__webui.templates[tag] = {
    h: '<div></div>',
  };
}

interface ComplexPropertyWriter {
  $writeComplexProperty(
    element: Element,
    name: string,
    value: unknown,
    replayAfterHydration: boolean,
  ): void;
}

interface PendingParentStateConsumer {
  $applyPendingParentState(replayAfterHydration: boolean): void;
}

describe('TemplateElement complex-property delivery', () => {
  test('queues an unresolved WebUI child without creating an own property', () => {
    const tag = 'test-pending-property';
    registerTemplate(tag);
    class PendingChild extends TemplateElement {
      protected override $observableNames(): Set<string> {
        return new Set(['payload']);
      }
    }
    Object.defineProperty(PendingChild.prototype, 'payload', {
      get(this: { _payload?: unknown }) {
        return this._payload;
      },
      set(this: { _payload?: unknown }, value: unknown) {
        this._payload = value;
      },
      configurable: true,
    });

    const parent = new TemplateElement();
    const child = new PendingChild() as PendingChild & {
      localName: string;
      _payload?: unknown;
    };
    child.localName = tag;

    (parent as unknown as ComplexPropertyWriter).$writeComplexProperty(
      child,
      'payload',
      { label: 'from parent' },
      false,
    );

    assert.equal(Object.hasOwn(child, 'payload'), false);
    (child as unknown as PendingParentStateConsumer)
      .$applyPendingParentState(false);
    assert.deepEqual(child._payload, { label: 'from parent' });
    assert.equal(Object.hasOwn(child, 'payload'), false);
  });

  test('preserves direct assignment for an unresolved third-party element', () => {
    const parent = new TemplateElement();
    const child = { localName: 'external-property-target' } as unknown as
      Element & { payload?: unknown };

    (parent as unknown as ComplexPropertyWriter).$writeComplexProperty(
      child,
      'payload',
      { label: 'direct' },
      false,
    );

    assert.deepEqual(child.payload, { label: 'direct' });
    assert.equal(Object.hasOwn(child, 'payload'), true);
  });
});

describe('TemplateElement.connectedCallback — streamed-host (data-ws) deferral', () => {
  test('defers a data-ws-marked streamed host without warning, even when metadata is missing', () => {
    resetStreamingModeForTests();

    const previousWarn = console.warn;
    let warned = false;
    console.warn = () => {
      warned = true;
    };

    try {
      // A streamed SSR component host carries the parser-emitted `data-ws`
      // marker. It connects at its opening tag (zero children, no shadow root)
      // before its boundary — and thus its template metadata — has streamed in.
      const el = new TemplateElement();
      (el as unknown as { tagName: string }).tagName = 'test-stream-widget';
      (el as unknown as { setAttribute(n: string, v: string): void }).setAttribute('data-ws', '');

      assert.equal((el as unknown as { childNodes: unknown[] }).childNodes.length, 0);

      el.connectedCallback();

      assert.equal(warned, false, 'a marked streamed host must never warn about in-flight metadata');
      assert.equal((el as unknown as { $deferredSSR: boolean }).$deferredSSR, true);
      assert.equal((el as unknown as { $ready: boolean }).$ready, true);
      // The marker stays until the boundary activates the instance.
      assert.equal((el as unknown as { hasAttribute(n: string): boolean }).hasAttribute('data-ws'), true);
    } finally {
      console.warn = previousWarn;
    }
  });

  test('lets a pending-definition resume own activation without replaying ordinary bootstrap state', () => {
    const tag = 'test-pending-resume-widget';
    registerTemplate(tag);
    let ordinaryDeferrals = 0;
    let received: Record<string, unknown> | undefined;

    class PendingResumeElement extends TemplateElement {
      protected override $didDeferSSRHydration(): void {
        ordinaryDeferrals++;
      }
    }

    const el = new PendingResumeElement();
    const raw = el as unknown as {
      tagName: string;
      $deferredSSR: boolean;
      $hydrated: boolean;
      setAttribute(name: string, value: string): void;
      removeAttribute(name: string): void;
      [PENDING_ROOT_CONNECTED]?: () => void;
      [STREAMING_BOUNDARY_ACTIVATE](
        state?: Record<string, unknown>,
      ): number;
    };
    raw.tagName = tag;
    raw.$hydrated = true;
    raw.setAttribute('data-ws', '');
    raw[PENDING_ROOT_CONNECTED] = () => {
      received = { status: 'ready' };
      assert.equal(
        raw[STREAMING_BOUNDARY_ACTIVATE](received),
        ACTIVATION_ACTIVATED,
      );
      raw.removeAttribute('data-ws');
    };

    el.connectedCallback();

    assert.deepEqual(received, { status: 'ready' });
    assert.equal(raw.$deferredSSR, false);
    assert.equal(
      ordinaryDeferrals,
      0,
      'ordinary deferral would replay older page bootstrap state after the queued update',
    );
  });

  test('warns for an UNMARKED client-created element with missing metadata (no silent defer)', () => {
    resetStreamingModeForTests();

    const previousWarn = console.warn;
    let warned = false;
    console.warn = () => {
      warned = true;
    };

    try {
      // No `data-ws`: a genuinely client-created empty element. With the old
      // empty-subtree heuristic removed, a missing template is now a real
      // authoring error surfaced immediately rather than an indefinite defer.
      const el = new TemplateElement();
      (el as unknown as { tagName: string }).tagName = 'test-unmarked-widget';

      el.connectedCallback();

      assert.equal(warned, true, 'an unmarked element with no metadata must warn, not defer');
      assert.notEqual((el as unknown as { $deferredSSR: boolean }).$deferredSSR, true);
    } finally {
      console.warn = previousWarn;
    }
  });

  test('does not reserve an authored data-ws attribute on an ordinary page', () => {
    const documentFake = document as unknown as {
      querySelector(selector: string): unknown;
    };
    const streamingQuery = documentFake.querySelector;
    documentFake.querySelector = () => null;
    resetStreamingModeForTests();

    const previousWarn = console.warn;
    let warned = false;
    console.warn = () => {
      warned = true;
    };

    try {
      const el = new TemplateElement();
      (el as unknown as { tagName: string }).tagName = 'test-ordinary-data-attribute';
      (el as unknown as { setAttribute(n: string, v: string): void }).setAttribute('data-ws', 'authored');

      el.connectedCallback();

      assert.equal(warned, true, 'ordinary lifecycle continues through metadata lookup');
      assert.notEqual((el as unknown as { $deferredSSR: boolean }).$deferredSSR, true);
    } finally {
      console.warn = previousWarn;
      documentFake.querySelector = streamingQuery;
      resetStreamingModeForTests();
    }
  });
});

describe('TemplateElement.connectedCallback — reused-template race', () => {
  test('defers a data-ws instance even when the tag metadata is already registered by an earlier boundary', () => {
    resetStreamingModeForTests();

    // Boundary 0 already committed and registered this tag's template metadata;
    // a later, not-yet-committed boundary connects its own <test-reused-widget>
    // instance whose SSR children have not been parsed yet. The `data-ws`
    // marker short-circuits the metadata lookup, so $mount() can never
    // misclassify the empty instance as client-created.
    registerTemplate('test-reused-widget');

    const el = new TemplateElement();
    (el as unknown as { tagName: string }).tagName = 'test-reused-widget';
    (el as unknown as { setAttribute(n: string, v: string): void }).setAttribute('data-ws', '');

    assert.equal((el as unknown as { childNodes: unknown[] }).childNodes.length, 0);

    el.connectedCallback();

    assert.equal((el as unknown as { $deferredSSR: boolean }).$deferredSSR, true);
    assert.equal((el as unknown as { $ready: boolean }).$ready, true);
    // $mount() must never have run: no client root wired, $meta still unset
    // (resolved lazily at activation).
    assert.equal((el as unknown as { $root: unknown }).$root, null);
    assert.equal((el as unknown as { $meta: unknown }).$meta, undefined);
  });
});

describe('TemplateElement — streamed-host activation ownership', () => {
  test('activation clears the deferral but leaves data-ws for the coordinator to strip', () => {
    resetStreamingModeForTests();
    registerTemplate('test-activate-widget');

    const el = new TemplateElement();
    (el as unknown as { tagName: string }).tagName = 'test-activate-widget';
    const raw = el as unknown as {
      setAttribute(n: string, v: string): void;
      hasAttribute(n: string): boolean;
      $deferredSSR: boolean;
      $hydrated: boolean;
      [STREAMING_BOUNDARY_ACTIVATE](state?: Record<string, unknown>): number;
    };
    raw.setAttribute('data-ws', '');

    // Reach the deferred state the coordinator would activate. Marking the
    // instance already-hydrated makes $mount() a clean no-op so this test
    // isolates the activation contract without a real DOM.
    el.connectedCallback();
    assert.equal(raw.$deferredSSR, true);
    assert.equal(raw.hasAttribute('data-ws'), true);
    raw.$hydrated = true;

    raw[STREAMING_BOUNDARY_ACTIVATE]();

    assert.equal(raw.$deferredSSR, false, 'activation clears the deferral');
    // Successful-path marker removal is centralized in the streaming
    // coordinator's `invokeActivationHook` (proved by the pipeline tests), NOT
    // duplicated in TemplateElement — so invoking the hook directly leaves the
    // marker in place. This guards against re-introducing the duplicate cleanup.
    assert.equal(raw.hasAttribute('data-ws'), true, 'TemplateElement itself does not strip the marker');
  });

  test('activation establishes deferral for a detached root upgraded before first connection', () => {
    let received: Record<string, unknown> | undefined;
    let wasDeferred = false;

    class DetachedUpgradeElement extends TemplateElement {
      protected override $activateDeferredSSR(state?: Record<string, unknown>): void {
        wasDeferred = (this as unknown as { $deferredSSR: boolean }).$deferredSSR;
        received = state;
      }
    }

    const el = new DetachedUpgradeElement();
    const raw = el as unknown as {
      $meta?: TemplateMeta;
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](state?: Record<string, unknown>): number;
    };
    raw.$meta = { h: '<span></span>' };
    raw.setAttribute('data-ws', '');

    raw[STREAMING_BOUNDARY_ACTIVATE]({ detached: true });

    assert.equal(wasDeferred, true, 'the marker establishes dormant SSR state without connectedCallback');
    assert.deepEqual(received, { detached: true });
  });

  test('a coordinator-resolved bypass ancestor is skipped exactly once', () => {
    const parentTag = 'test-spanning-parent';
    const childTag = 'test-early-span-child';
    registerTemplate(parentTag);
    registerTemplate(childTag);

    const parent = new TemplateElement();
    const parentRaw = parent as unknown as {
      tagName: string;
      parentElement: Element | null;
      $deferredSSR: boolean;
    };
    parentRaw.tagName = parentTag;
    parentRaw.parentElement = null;
    parentRaw.$deferredSSR = true;

    const child = new TemplateElement();
    const childRaw = child as unknown as {
      tagName: string;
      parentElement: Element;
      $deferredSSR: boolean;
      $hydrated: boolean;
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](
        state?: Record<string, unknown>,
        bypassAncestor?: Element,
      ): number;
    };
    childRaw.tagName = childTag;
    childRaw.parentElement = parent as unknown as Element;
    childRaw.setAttribute('data-ws', '');
    child.connectedCallback();
    childRaw.$hydrated = true;

    assert.equal(
      childRaw[STREAMING_BOUNDARY_ACTIVATE](
        { child: true },
        parent as unknown as Element,
      ),
      ACTIVATION_ACTIVATED,
    );
    assert.equal(childRaw.$deferredSSR, false);
  });

  test('an unrelated bypass ancestor preserves the parent-first barrier', () => {
    const parentTag = 'test-mismatch-span-parent';
    const childTag = 'test-mismatch-span-child';
    registerTemplate(parentTag);
    registerTemplate(childTag);

    const parent = new TemplateElement();
    const parentRaw = parent as unknown as {
      tagName: string;
      parentElement: Element | null;
      $deferredSSR: boolean;
    };
    parentRaw.tagName = parentTag;
    parentRaw.parentElement = null;
    parentRaw.$deferredSSR = true;

    const unrelated = new TemplateElement();
    (unrelated as unknown as { tagName: string }).tagName = parentTag;

    const child = new TemplateElement();
    const childRaw = child as unknown as {
      tagName: string;
      parentElement: Element;
      $deferredSSR: boolean;
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](
        state?: Record<string, unknown>,
        bypassAncestor?: Element,
      ): number;
    };
    childRaw.tagName = childTag;
    childRaw.parentElement = parent as unknown as Element;
    childRaw.setAttribute('data-ws', '');
    child.connectedCallback();

    assert.equal(
      childRaw[STREAMING_BOUNDARY_ACTIVATE](
        { child: true },
        unrelated as unknown as Element,
      ),
      ACTIVATION_ANCESTOR_BARRIER,
    );
    assert.equal(childRaw.$deferredSSR, true);
  });

  test('only one barrier is bypassed when barriers nest', () => {
    const outerTag = 'test-nested-bypass-outer';
    const innerTag = 'test-nested-bypass-inner';
    const childTag = 'test-nested-bypass-child';
    registerTemplate(outerTag);
    registerTemplate(innerTag);
    registerTemplate(childTag);

    const outer = new TemplateElement();
    const outerRaw = outer as unknown as {
      tagName: string;
      parentElement: Element | null;
      $deferredSSR: boolean;
    };
    outerRaw.tagName = outerTag;
    outerRaw.parentElement = null;
    outerRaw.$deferredSSR = true;

    const inner = new TemplateElement();
    const innerRaw = inner as unknown as {
      tagName: string;
      parentElement: Element | null;
      $deferredSSR: boolean;
    };
    innerRaw.tagName = innerTag;
    innerRaw.parentElement = outer as unknown as Element;
    innerRaw.$deferredSSR = true;

    const child = new TemplateElement();
    const childRaw = child as unknown as {
      tagName: string;
      parentElement: Element;
      $deferredSSR: boolean;
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](
        state?: Record<string, unknown>,
        bypassAncestor?: Element,
      ): number;
    };
    childRaw.tagName = childTag;
    childRaw.parentElement = inner as unknown as Element;
    childRaw.setAttribute('data-ws', '');
    child.connectedCallback();

    // `inner` is stepped over, but `outer` is still an unfinished barrier.
    assert.equal(
      childRaw[STREAMING_BOUNDARY_ACTIVATE](
        { child: true },
        inner as unknown as Element,
      ),
      ACTIVATION_ANCESTOR_BARRIER,
    );
    assert.equal(childRaw.$deferredSSR, true);
  });

  test('authored components do not globally defer unmarked SSR-shaped light DOM', () => {
    const el = new TemplateElement() as unknown as {
      $shouldDeferSSRHydration(): boolean;
    };
    assert.equal(el.$shouldDeferSSRHydration(), false);
  });

  test('reports missing metadata numerically and explicitly abandons internal deferral', () => {
    const el = new TemplateElement();
    const raw = el as unknown as {
      tagName: string;
      $deferredSSR: boolean;
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](state?: Record<string, unknown>): number;
      [STREAMING_BOUNDARY_ABANDON](): void;
    };
    raw.tagName = 'test-missing-activation-meta';
    raw.setAttribute('data-ws', '');
    el.connectedCallback();

    assert.equal(raw[STREAMING_BOUNDARY_ACTIVATE](), ACTIVATION_MISSING_TEMPLATE);
    assert.equal(raw.$deferredSSR, true);

    raw[STREAMING_BOUNDARY_ABANDON]();
    assert.equal(raw.$deferredSSR, false);
  });

  test('caches metadata before static-host opt-out so a later state write can wake it', () => {
    let activationMeta: TemplateMeta | undefined;
    class OptOutElement extends TemplateElement {
      protected override $shouldActivateOnBoundaryCommit(): boolean {
        return false;
      }

      protected override $afterExternalStateWrite(applied: boolean): void {
        if (applied) this.$activateDeferredSSR();
      }

      protected override $activateDeferredSSR(): void {
        activationMeta = (this as unknown as { $meta?: TemplateMeta }).$meta;
      }
    }
    const el = new OptOutElement();
    const raw = el as unknown as {
      tagName: string;
      $meta?: TemplateMeta;
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](): number;
    };
    raw.tagName = 'test-static-opt-out';
    registerTemplate(raw.tagName);
    window.__webui!.templates![raw.tagName].tr = ['message'];
    raw.setAttribute('data-ws', '');
    el.connectedCallback();

    assert.equal(raw[STREAMING_BOUNDARY_ACTIVATE](), ACTIVATION_STATIC_HOST_OPT_OUT);
    assert.ok(raw.$meta, 'boundary commit caches metadata without mounting');
    el.setState({ message: 'wake' });
    assert.equal(activationMeta, raw.$meta, 'the later state write can activate from cached metadata');
  });
});

describe('TemplateElement.define — ordinary (non-streaming) authored definition', () => {
  test('defers an eagerly authored define() until later-registered metadata completes it, even outside streaming mode', () => {
    // Ordinary WebUI Router partial navigation eagerly imports an authored
    // nested component module — which calls the compiler-emitted
    // `MyComponent.define(tag)` at top level — *before* the router has
    // registered that route's compiled template metadata. Native
    // `customElements.define()` snapshots `observedAttributes` at call time,
    // so defining now (as the old streaming-only guard did) would forever
    // miss template-only attributes like `title` below.
    const documentFake = document as unknown as {
      querySelector(selector: string): unknown;
    };
    const previousQuerySelector = documentFake.querySelector;
    documentFake.querySelector = () => null; // no streaming meta tag present
    resetStreamingModeForTests();

    const customElementsFake = customElements as unknown as {
      get(name: string): CustomElementConstructor | undefined;
      define?(name: string, ctor: CustomElementConstructor): void;
    };
    const previousGet = customElementsFake.get;
    const previousDefine = customElementsFake.define;
    const registry = new Map<string, CustomElementConstructor>();
    customElementsFake.get = (name: string) => registry.get(name);
    customElementsFake.define = (name: string, ctor: CustomElementConstructor) => {
      registry.set(name, ctor);
    };

    const tag = `test-router-nested-widget-${Date.now()}`;

    try {
      // No `@attr`/`@observable` for `title` — a template-only binding
      // entirely owned by compiled metadata (`tr`/`ta`).
      class AuthoredNestedWidget extends TemplateElement {}
      const AuthoredNestedWidgetCtor = AuthoredNestedWidget as unknown as {
        define(tagName: string): void;
      };

      AuthoredNestedWidgetCtor.define(tag);

      assert.equal(
        customElements.get(tag),
        undefined,
        'must stay pending — an ordinary (non-streaming) page must not define an incomplete observer surface either',
      );

      // A second eager define() call for the same still-pending tag must
      // preserve the existing duplicate-definition diagnostic.
      assert.throws(
        () => AuthoredNestedWidgetCtor.define(tag),
        /already pending definition/,
      );

      // The router registers this route's compiled template metadata once
      // its partial navigation response resolves.
      registerTemplateData({
        [tag]: {
          h: '<div><span></span></div>',
          tr: ['title'],
          ta: ['title'],
        },
      });

      const ctor = customElements.get(tag) as (CustomElementConstructor & {
        observedAttributes?: string[];
      }) | undefined;
      assert.ok(ctor, 'the deferred definition must complete once metadata registers');
      assert.deepEqual(
        ctor!.observedAttributes,
        ['title'],
        'observedAttributes must include the template-derived attribute the browser would otherwise have missed',
      );

      const el = new (ctor as CustomElementConstructor)() as unknown as {
        attributeChangedCallback(name: string, oldValue: string | null, newValue: string | null): void;
        $templateState: Record<string, unknown> | null;
      };

      // A client-created host now carries `title` in `observedAttributes`,
      // so the browser calls `attributeChangedCallback` for it.
      el.attributeChangedCallback('title', null, 'Hello from router nav');

      assert.equal(
        el.$templateState?.title,
        'Hello from router nav',
        'the template-only binding must receive and update from the client-created host attribute',
      );
    } finally {
      documentFake.querySelector = previousQuerySelector;
      resetStreamingModeForTests();
      customElementsFake.get = previousGet;
      customElementsFake.define = previousDefine;
    }
  });
});

describe('TemplateElement — scoped state availability', () => {
  test('uses scope knownness when an item value is explicitly undefined', () => {
    const el = new TemplateElement();

    assert.equal(
      el.$hasStateRoot('item', {
        name: 'item',
        value: undefined,
        known: true,
      }),
      true,
    );
    assert.equal(
      el.$hasStateRoot('item.label', {
        name: 'item',
        value: 'trusted SSR value',
        known: false,
      }),
      false,
    );
  });
});

describe('TemplateElement — hydration lifecycle exceptions', () => {
  test('a real streamed activation throw balances lifecycle and does not wedge terminal completion', () => {
    class ThrowingStateElement extends TemplateElement {
      protected override $shouldApplySSRBootstrapState(): boolean {
        throw new Error('state hydration failed');
      }
    }

    const tag = 'test-throwing-hydration-widget';
    registerTemplate(tag);
    __resetLifecycleForTests();
    dispatchedEvents.length = 0;
    beginStreamingGate();
    markBoundaryPending();

    const el = new ThrowingStateElement();
    const raw = el as unknown as {
      tagName: string;
      childNodes: unknown[];
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](state?: Record<string, unknown>): number;
    };
    raw.tagName = tag;
    raw.childNodes.push({});
    raw.setAttribute('data-ws', '');
    el.connectedCallback();

    assert.throws(
      () => raw[STREAMING_BOUNDARY_ACTIVATE]({ count: 1 }),
      /state hydration failed/,
      'TemplateElement must not swallow the hydration error',
    );
    let state = __getLifecycleStateForTests();
    assert.equal(state.pendingCount, 0, 'the throwing activation balances hydrationStart');
    assert.equal(state.completed, false, 'terminal has not committed yet');

    markBoundaryCommitted(true);
    state = __getLifecycleStateForTests();
    assert.equal(state.completed, true, 'balanced accounting allows terminal completion');
    assert.equal(dispatchedEvents.includes('webui:hydration-complete'), true);
  });

  test('a reconnect update throw also balances hydration lifecycle', () => {
    class ThrowingReconnectElement extends TemplateElement {
      override $update(): void {
        throw new Error('reconnect update failed');
      }
    }

    __resetLifecycleForTests();
    dispatchedEvents.length = 0;
    const el = new ThrowingReconnectElement();
    const raw = el as unknown as {
      tagName: string;
      $hydrated: boolean;
      $root: object;
    };
    raw.tagName = 'test-throwing-reconnect-widget';
    raw.$hydrated = true;
    raw.$root = {};

    assert.throws(() => el.connectedCallback(), /reconnect update failed/);
    const state = __getLifecycleStateForTests();
    assert.equal(state.pendingCount, 0);
    assert.equal(state.completed, true);
  });
});
