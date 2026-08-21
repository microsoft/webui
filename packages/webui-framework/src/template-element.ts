// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * TemplateElement — lightweight compiled-template custom element core.
 *
 * Supports Shadow DOM or light DOM, SSR hydration, reactive updates, and compiled
 * SSR content is reused by matching existing DOM nodes through compiled
 * template path mapping.  Client-created components use exact childNode
 * indices from the compiled template HTML.
 *
 * This module deliberately excludes decorator, event, ref, and custom-event
 * emitter code so authored components can use compiled-template hydration
 * without paying for features they do not use.
 *
 * ## SSR hydration markers
 *
 * The server-side handler plugin emits lightweight HTML comment markers
 * around structural boundaries so the client can hydrate in-place:
 *
 *   - `<!--wr-->` / `<!--/wr-->` — repeat (for-loop) block boundaries
 *   - `<!--wi-->` — repeat item boundary (one per item)
 *   - `<!--wc-->` / `<!--/wc-->` — conditional (if) block boundaries
 *
 * During hydration these markers are consumed: `<!--wi-->`, `<!--/wr-->`,
 * and `<!--/wc-->` are removed from the DOM.  The `<!--wr-->` start
 * marker is kept as the runtime repeat anchor, and `<!--wc-->` is kept
 * as the runtime condition anchor.
 *
 * **Marker removal is deferred** until after all path-based resolution
 * (`buildSSRIndex`, `$findSSRText`, `$finalize`) is complete.  This is
 * critical because both use marker pairs to skip structural
 * block content when counting element/text ordinals — removing a closing
 * marker mid-hydration would break later resolution calls.
 *
 * ## Comment anchors (client-created)
 *
 * For client-created components (no SSR), the framework inserts empty
 * comment nodes (`document.createComment('')`) as stable DOM anchors
 * for conditional and repeat blocks.  When SSR markers are absent,
 * the same fallback anchors are used.
 *
 * These comments are invisible to the user, weigh ~0 bytes, and are the
 * minimum DOM structure needed for the framework to operate.
 */

import { deferTemplateDefinition, getTemplate } from './template.js';
import {
  cloneTemplateContent,
} from './template-content.js';
import type {
  TemplateMeta,
  TemplateBlockMeta,
  CompiledAttrMeta,
  CompiledAttrPart,
  CompiledCondition,
  TemplateNodeIndex,
} from './template.js';
import { hydrationStart, hydrationEnd } from './lifecycle.js';
import {
  isStreamingHydrationMode,
  PENDING_ROOT_CONNECTED,
  STREAMED_HOST_ATTR,
  STREAMING_BOUNDARY_ACTIVATE,
} from './streaming-mode.js';
import {
  createRepeatKeyState,
  seedHydratedRepeatKeys,
  syncRepeat,
  dotWalk,
} from './element/diff.js';
import {
  collectItemMarkers,
  nextElement,
  findByOrdinal,
  buildSSRIndex,
  collectTemplateElements,
  MARKER_COND_START,
  MARKER_COND_END,
} from './element/markers.js';
import {
  claimSsrComponentStyles,
  installComponentStyles,
  isComponentStyleMarker,
} from './element/styles.js';
import {
  cancelTemplateLinkStyleMount,
  installTemplateLinkStyles,
  prepareTemplateLinkStyles,
  templateMayContainLinkStyles,
} from './element/link-styles.js';
import {
  ATTR_KIND_BOOLEAN,
  ATTR_KIND_COMPLEX,
  ATTR_KIND_TEMPLATE,
  hasNativeLiveProperty,
} from './element/types.js';
import { templateHasRoot, templateRootForAttribute } from './template-roots.js';
import type {
  AttrBinding,
  CondBinding,
  RepeatBinding,
  ScopeFrame,
  TemplateInstance,
  TextBinding,
} from './element/types.js';
// Type-only import: erased at compile time, so it never creates a runtime edge
// to the diagnostic module. That module's runtime code is reached solely through
// the dynamic `import()` in `$checkHydrationMismatch`, which is what lets a
// production bundler drop it (see the `__WEBUI_DEV__` note below).
import type { MismatchContext } from './hydration-mismatch.js';

// ── Development build flag ──────────────────────────────────────
// `__WEBUI_DEV__` is a compile-time constant a bundler folds to a literal
// (`esbuild --define:__WEBUI_DEV__=false`, webpack/rspack `DefinePlugin`, Vite
// and Rollup/rolldown `define`, swc `globals.vars`). `webui-press` folds it to
// `false` for `build` (production) and leaves it undefined for `serve` (dev).
//
// When it folds to `false`, `DEV` folds too: every `if (DEV)` / `if (!DEV)
// return` branch below becomes dead code, and the *sole* dynamic `import()` of
// `hydration-mismatch.ts` is DCE'd. Because that import is the only reference to
// the module, dropping it orphans the chunk and the bundler removes the whole
// diagnostic (comparators + message string) from the output — not just its
// runtime cost. A static import would NOT strip: esbuild fixes module
// reachability before constant-folding and never re-runs tree-shaking, so a
// statically-imported module survives even when its only caller is folded away.
//
// The `typeof` guard keeps this safe when the flag is undefined (raw ESM, the
// framework's own `tsc` output, unit tests, un-defined esbuild builds): `typeof`
// on an undeclared identifier yields `"undefined"` instead of throwing a
// ReferenceError, so `DEV` defaults to `true` (diagnostics on). Declared and
// consumed module-locally on purpose — esbuild folds a module-local `const`
// reliably, whereas an imported constant is not inlined across module
// boundaries, which would defeat the stripping.
declare const __WEBUI_DEV__: boolean;
const DEV: boolean = typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__;

// ── Caches ──────────────────────────────────────────────────────

/** Cached root tag name extracted from meta.h before it's released. */
const rootTagCache = new WeakMap<TemplateBlockMeta, string | null>();

/** Parsed template DOM for SSR path mapping, keyed by TemplateBlockMeta. */
const templateDOMCache = new WeakMap<TemplateBlockMeta, Element>();

/** Pre-computed ordinals for template nodes: childIndex → [nodeType, ordinal].
 *  Avoids re-counting siblings on every text-slot resolution. */
const tplOrdinalCache = new WeakMap<Node, Map<number, [nodeType: number, ordinal: number]>>();

/**
 * Pre-order element table for a parsed template, keyed by its root.
 *
 * The parsed template DOM is cached per metadata object, so this is built once
 * per component type and shared by every instance.
 */
const tplElementCache = new WeakMap<Node, Array<Node | undefined>>();

function getTemplateElements(tplRoot: Node): Array<Node | undefined> {
  let cached = tplElementCache.get(tplRoot);
  if (!cached) {
    cached = collectTemplateElements(tplRoot);
    tplElementCache.set(tplRoot, cached);
  }
  return cached;
}

function getTplOrdinals(tplNode: Node): Map<number, [number, number]> {
  let map = tplOrdinalCache.get(tplNode);
  if (map) return map;
  map = new Map();
  let elemOrd = 0;
  let textOrd = 0;
  const children = tplNode.childNodes;
  for (let k = 0; k < children.length; k++) {
    const type = children[k].nodeType;
    if (type === 1) { map.set(k, [1, elemOrd]); elemOrd++; }
    else if (type === 3) { map.set(k, [3, textOrd]); textOrd++; }
  }
  tplOrdinalCache.set(tplNode, map);
  return map;
}

// ── Sentinels ───────────────────────────────────────────────────

const EMPTY_ARR: readonly never[] = [];
const EMPTY_SET: Set<string> = Object.freeze(new Set<string>()) as Set<string>;
/** Branded single-key state writer used by framework bindings, not public duck typing. */
const WEBUI_SET_STATE_KEY = Symbol.for('microsoft.webui.setStateKey');
/** Branded fatal-stream cleanup hook, invoked before `data-ws` is removed. */
const STREAMING_BOUNDARY_ABANDON = Symbol.for('microsoft.webui.boundaryAbandon');
const ACTIVATION_ACTIVATED = 1;
const ACTIVATION_STATIC_HOST_OPT_OUT = 2;
const ACTIVATION_MISSING_TEMPLATE = 3;

const templateMetaByCtor = new WeakMap<Function, TemplateMeta>();
const pendingAncestorDescendants = new WeakMap<Element, TemplateElement[]>();

interface PendingParentState {
  readonly values: Record<string, unknown>;
  replay?: Set<string>;
}

const pendingParentStateByElement = new WeakMap<Element, PendingParentState>();

function queuePendingParentState(
  element: Element,
  name: string,
  value: unknown,
  replayAfterHydration: boolean,
): void {
  let pending = pendingParentStateByElement.get(element);
  if (!pending) {
    pending = {
      values: Object.create(null) as Record<string, unknown>,
    };
    pendingParentStateByElement.set(element, pending);
  }
  pending.values[name] = value;
  if (replayAfterHydration) {
    (pending.replay ??= new Set()).add(name);
  }
}

function isUnupgradedWebUITarget(element: Element): boolean {
  const tagName = element.localName;
  if (tagName.indexOf('-') === -1) return false;
  const ctor = customElements.get(tagName);
  if (ctor) return ctor.prototype instanceof TemplateElement;
  return getTemplate(tagName) !== undefined;
}

type TemplateObservedConstructor = CustomElementConstructor & {
  readonly observedAttributes?: readonly string[];
};

// ── Helper: snapshot child nodes into a pre-allocated array ──────

function childNodesArray(parent: Node): Node[] {
  const children = parent.childNodes;
  const len = children.length;
  const result = new Array<Node>(len);
  for (let i = 0; i < len; i++) result[i] = children[i];
  return result;
}

// ── Helper: parse template HTML into a temp container ────────────

function getTemplateDom(meta: TemplateBlockMeta): Element {
  let cached = templateDOMCache.get(meta);
  if (cached) return cached;
  const div = document.createElement('div');
  div.innerHTML = meta.h;
  templateDOMCache.set(meta, div);
  return div;
}

/**
 * Merge authored `observedAttributes` with template-read roots.
 *
 * This is the key that makes `@attr` optional for HTML-only values: if a template
 * reads `title`, then a host `title="..."` mutation can update the hidden
 * template state even when the class never declared an `@attr title` property.
 */
function installTemplateObservedAttributes(
  ctor: TemplateObservedConstructor,
  tagName: string,
  meta = getTemplate(tagName),
): void {
  if (!meta) return;

  templateMetaByCtor.set(ctor, meta);
  const attrs = meta.ta ?? EMPTY_ARR;
  if (attrs.length === 0) return;

  const existing = ctor.observedAttributes ?? EMPTY_ARR;
  const merged = new Array<string>(existing.length + attrs.length);
  let count = 0;
  for (let i = 0; i < existing.length; i++) {
    merged[count] = existing[i];
    count += 1;
  }
  for (let attrIndex = 0; attrIndex < attrs.length; attrIndex++) {
    const attrName = attrs[attrIndex];
    let found = false;
    for (let i = 0; i < count; i++) {
      if (merged[i] === attrName) {
        found = true;
        break;
      }
    }
    if (!found) {
      merged[count] = attrName;
      count += 1;
    }
  }
  merged.length = count;

  Object.defineProperty(ctor, 'observedAttributes', {
    get() {
      return merged;
    },
    configurable: true,
  });
}

function defineTemplateConstructor(
  ctor: TemplateObservedConstructor,
  tagName: string,
  meta?: TemplateMeta,
): void {
  installTemplateObservedAttributes(ctor, tagName, meta);
  if (meta) {
    const ready = prepareTemplateLinkStyles(meta);
    if (ready) void ready;
  }
  customElements.define(tagName, ctor);
}

/**
 * Return true for properties intentionally provided by component code.
 *
 * Hidden template state must not shadow authored fields, accessors, or methods.
 * The scan stops at `TemplateElement.prototype`, so native `HTMLElement` properties
 * like `title` and `id` do not block template-only state roots with those names.
 */
function hasAuthoredMember(instance: object, key: string): boolean {
  if (Object.prototype.hasOwnProperty.call(instance, key)) return true;

  let proto = Object.getPrototypeOf(instance) as object | null;
  while (proto && proto !== TemplateElement.prototype) {
    if (Object.prototype.hasOwnProperty.call(proto, key)) return true;
    proto = Object.getPrototypeOf(proto) as object | null;
  }
  return false;
}

// ═══════════════════════════════════════════════════════════════════
//  TemplateElement - compiled rendering core (no decorators / events / refs / emit)
// ═══════════════════════════════════════════════════════════════════

/**
 * Compiled WebUI rendering core.
 *
 * This class hydrates SSR output, creates client-side template instances, keeps
 * template-only state for omitted `@observable` / `@attr` fields, and updates
 * DOM bindings. It deliberately contains no decorator, event, ref, or
 * custom-event emitter code so authored components have the smallest
 * reachable runtime.
 */
export class TemplateElement extends HTMLElement {
  private static readonly $ancestorReleaseQueue: TemplateElement[] = [];
  private static $ancestorReleaseIndex = 0;
  private static $ancestorReleaseActive = false;

  private $root: TemplateInstance | null = null;
  private $meta?: TemplateMeta;
  private $ready = false;
  private $hydrated = false;
  private $deferredClientMount = false;
  private $resetClientShadow = false;
  /** Retained across teardown so reconnect is never mistaken for fresh SSR. */
  private $hasMounted = false;
  private $deferredSSR = false;
  private $activatingDeferredSSR = false;
  declare private $deferredAncestor: TemplateElement | undefined;
  declare private $pendingAncestor: Element | undefined;
  declare private $deferredDescendants: TemplateElement[] | undefined;
  declare private $deferredByAncestor: boolean | undefined;
  declare private $ancestorBoundaryState:
    | Record<string, unknown>
    | undefined;
  declare private $hasAncestorBoundaryState: boolean | undefined;
  /** True once a repeat produced an SSR item scope whose collection state is
   *  absent on the client. Only these instances need the per-binding
   *  scope-known walk in `$updateBindings`; authored components never set this,
   *  so their update loop skips the walk entirely. */
  private $hasUnknownScopes = false;
  private $templateState: Record<string, unknown> | null = null;
  private $dirtyPaths: Set<string> | null = null;
  private $pendingFlush = false;
  /** Observable paths written while connected but before hydration finished
   *  (constructor, `@observable` field initializer, or before
   *  `super.connectedCallback()`). Checked against the SSR DOM at `$ready` to
   *  surface hydration mismatches (issue #379). Stays `null` for components
   *  that follow the lifecycle, so the common path allocates nothing. */
  private $preReadyWrites: Set<string> | null = null;
  /** State roots written while lazy SSR hydration is deferred. The values live
   *  in normal authored/template state; this set prevents older bootstrap state
   *  from replacing them and requests one synchronous replay after wiring. */
  declare private $deferredWrites: Set<string> | undefined;
  /** Keep unavailable template-only roots from being replaced by empty values
   *  after a deferred SSR activation. */
  declare private $guardUnknownState: boolean | undefined;
  /** Cached condition resolver — avoids allocating a closure per evaluation. */
  private $resolver = (p: string, s?: unknown): unknown => this.$resolveValue(p, s as ScopeFrame | undefined);
  private $pathIndex?: Map<string, {
    texts: TextBinding[];
    attrs: AttrBinding[];
    conds: CondBinding[];
    repeats: RepeatBinding[];
  }>;
  /** Bindings that reference non-observable paths — updated on every flush. */
  private $wildcardBindings?: {
    texts: TextBinding[];
    attrs: AttrBinding[];
    conds: CondBinding[];
    repeats: RepeatBinding[];
  } | null;

  /** Internal single-key state hook used by compiled parent-to-child bindings. */
  [WEBUI_SET_STATE_KEY](key: string, value: unknown): boolean {
    const wasDeferred = this.$deferredSSR;
    this.$beforeExternalStateWrite();
    const owned = this.$setStateKey(key, value);
    if (owned && wasDeferred) this.$recordDeferredWrite(key);
    this.$afterExternalStateWrite(owned);
    return owned;
  }

  /**
   * Internal hook invoked by the streaming coordinator (`streaming.ts`) once
   * a boundary containing this element has committed. Returns a numeric outcome
   * so the allocation-sensitive coordinator can distinguish activation, an
   * intentional static-host opt-out, and missing metadata without per-root
   * result objects. The optional `state` is this element's boundary-local SSR
   * state, handed straight through to hydration instead of via the global
   * `window.__webui.state` handoff.
   */
  [STREAMING_BOUNDARY_ACTIVATE](state?: Record<string, unknown>): number {
    // `customElements.upgrade()` installs this class on detached roots without
    // invoking connectedCallback(). Preserve the same marker-driven dormant
    // state those roots would have entered while connected before activation.
    if (!this.$deferredSSR && this.hasAttribute(STREAMED_HOST_ATTR)) {
      this.$deferredSSR = true;
      this.$ready = true;
    }
    if (!this.$deferredSSR) return ACTIVATION_ACTIVATED;
    // Report missing metadata explicitly. A silent no-op would let the
    // coordinator publish completion while this root remained inert.
    const meta = this.$templateMeta();
    if (!meta) {
      return ACTIVATION_MISSING_TEMPLATE;
    }
    this.$meta = meta;
    if (!this.$shouldActivateOnBoundaryCommit()) {
      // A static host never activates, so this is the only chance to install
      // its styles.
      this.$installStyles(meta);
      return ACTIVATION_STATIC_HOST_OPT_OUT;
    }
    const ancestor = this.$nearestHydrationBarrier();
    if (ancestor) {
      this.$deferredByAncestor = true;
      this.$ancestorBoundaryState = state;
      this.$hasAncestorBoundaryState = true;
      this.$registerWithHydrationBarrier(ancestor);
      return ACTIVATION_ACTIVATED;
    }
    this.$activatingDeferredSSR = true;
    try {
      this.$activateDeferredSSR(state);
    } finally {
      this.$activatingDeferredSSR = false;
    }
    return ACTIVATION_ACTIVATED;
  }

  /** Clear element-owned streaming deferral after a fatal stream failure. */
  [STREAMING_BOUNDARY_ABANDON](): void {
    this.$detachDeferredAncestor();
    this.$abandonDeferredDescendants();
    this.$deferredSSR = false;
    this.$deferredByAncestor = undefined;
    this.$ancestorBoundaryState = undefined;
    this.$hasAncestorBoundaryState = undefined;
    this.$activatingDeferredSSR = false;
    this.$ready = false;
    this.$preReadyWrites = null;
    if (this.$deferredWrites) this.$deferredWrites = undefined;
  }

  /**
   * Register this constructor for a tag and install template-derived observers.
   */
  static define(tagName: string): void {
    const ctor = this as TemplateObservedConstructor;
    const meta = getTemplate(tagName);
    if (!meta && isStreamingHydrationMode()) {
      // Metadata supplies template-derived observed attributes. The browser
      // snapshots that list at native define() time, so a streamed page must
      // wait rather than permanently define an incomplete observer surface.
      deferTemplateDefinition(tagName, ctor, () => {
        defineTemplateConstructor(ctor, tagName, getTemplate(tagName));
      });
      return;
    }
    defineTemplateConstructor(ctor, tagName, meta);
  }

  // ── Lifecycle ─────────────────────────────────────────────────

  connectedCallback(): void {
    const tag = this.tagName.toLowerCase();
    this.$adoptPendingDescendants();

    if (this.$deferredSSR) {
      this.$ready = true;
      if (this.$deferredByAncestor) {
        const ancestor = this.$nearestHydrationBarrier();
        if (ancestor) {
          this.$registerWithHydrationBarrier(ancestor);
          return;
        }
        this.$detachDeferredAncestor();
        this.$releaseAncestorBarrier();
        return;
      }
      this.$didDeferSSRHydration();
      return;
    }

    if (this.$hydrated && this.$root) {
      if (this.$deferredClientMount) return;
      hydrationStart();
      try {
        this.$ready = true;
        this.$update();
      } finally {
        hydrationEnd();
      }
      return;
    }

    // Streamed SSR hosts carry an explicit `data-ws` marker emitted by the
    // server/parser on every streamed component host root (and only those). It
    // is the sole signal that this element's SSR subtree is still being
    // streamed and its boundary has not committed yet: defer, and let the
    // streaming coordinator (`streaming.ts`) activate this instance once its
    // boundary commits — by which point its subtree has fully parsed. It also
    // resolves the reused-tag race (an earlier boundary already registered
    // this tag's metadata, but the parser has not yet appended *this*
    // instance's own SSR children): $mount() would misclassify it as
    // client-created, so deferring on the marker sidesteps that entirely.
    //
    // A genuinely client-created empty element made during the streaming
    // window has no `data-ws` and falls through to mount normally below — the
    // marker, not an empty-subtree heuristic, is what distinguishes the two.
    if (
      !this.$hasMounted &&
      isStreamingHydrationMode() &&
      this.hasAttribute(STREAMED_HOST_ATTR)
    ) {
      this.$deferredSSR = true;
      this.$ready = true;
      const resume = (this as unknown as { [PENDING_ROOT_CONNECTED]?: () => void })[PENDING_ROOT_CONNECTED];
      if (typeof resume === 'function') resume.call(this);
      this.$didDeferSSRHydration();
      return;
    }

    const meta = this.$templateMeta();
    if (!meta) {
      console.warn(
        `[WebUI] Template metadata for <${tag}> not found. ` +
        `Ensure the component is included in the SSR output or registered via __webui.templates.`,
      );
      return;
    }
    this.$meta = meta;
    // Under WebUI's loading contract, deferred scripts run after parsing and
    // blocking scripts follow every component instance they may upgrade.
    // Mount synchronously so super.connectedCallback() is the hydration boundary.
    if (this.$hasMounted) {
      this.$remount(meta);
    } else {
      this.$mount(meta, false);
    }
  }

  /** Rewire retained DOM, or rebuild structural DOM, from current client state. */
  private $remount(meta: TemplateMeta): void {
    this.$mount(meta, false, undefined, false, true);
  }

  /** Mount the component after children are available. */
  private $mount(
    meta: TemplateMeta,
    forceSSR: boolean,
    ssrState?: Record<string, unknown>,
    hasBoundaryState = false,
    reconnecting = false,
  ): void {
    if (this.$hydrated) return;

    // Auto-detect shadow vs light DOM
    const hasShadow = !!this.shadowRoot;
    const wantShadow = hasShadow || !!meta.sd;
    const resetClientShadow = this.$resetClientShadow;
    this.$resetClientShadow = false;
    const remountStructuralTemplate = reconnecting &&
      ((meta.c?.length ?? 0) !== 0 || (meta.r?.length ?? 0) !== 0);

    let root: Node;
    let isSSR: boolean;
    let clientRoot: HTMLElement | null = null;

    if (remountStructuralTemplate) {
      const renderRoot = wantShadow
        ? this.shadowRoot ?? this.attachShadow({ mode: 'open' })
        : this;
      const retainedStyles: Element[] = [];
      let child = renderRoot.firstElementChild;
      while (child) {
        if (isComponentStyleMarker(child)) retainedStyles.push(child);
        child = child.nextElementSibling;
      }
      renderRoot.replaceChildren(...retainedStyles);
      root = renderRoot;
      isSSR = false;
    } else if (hasShadow && !resetClientShadow) {
      // Shadow DOM SSR — declarative shadow root already has content
      root = this.shadowRoot!;
      isSSR = true;
    } else if (this.childNodes.length > 0 && !meta.sd) {
      // SSR light-DOM — element already has server-rendered children.
      // Only treat as SSR when the template does NOT explicitly declare
      // shadow DOM (meta.sd).  When meta.sd is set, existing children
      // are slot content from an SPA partial, not SSR output.
      root = this;
      isSSR = true;
    } else if (wantShadow) {
      // Shadow DOM client-created (or SPA partial with slot content).
      // Existing children are slot content — they stay in light DOM
      // and project through the template's <slot>.
      const renderRoot = this.shadowRoot ?? this.attachShadow({ mode: 'open' });
      if (resetClientShadow) renderRoot.replaceChildren();
      root = renderRoot;
      isSSR = false;
    } else {
      // Light DOM client-created — populate from template (no shadow = no link issue)
      root = this;
      isSSR = false;
    }

    // Styles are required even when a compiler-owned SSR host remains dormant.
    // Install them after root selection but before the hydration deferral.
    this.$installStyles(meta);

    if (isSSR && !forceSSR && !reconnecting) {
      const ancestor = this.$nearestHydrationBarrier();
      if (ancestor) {
        this.$meta = meta;
        this.$deferredSSR = true;
        this.$deferredByAncestor = true;
        this.$ready = true;
        this.$primeSSRStateForDeferral();
        this.$registerWithHydrationBarrier(ancestor);
        return;
      }
      if (this.$shouldDeferSSRHydration(meta)) {
        this.$meta = meta;
        this.$deferredSSR = true;
        this.$ready = true;
        this.$didDeferSSRHydration();
        return;
      }
    }

    let deferredHydrationFinish = false;
    hydrationStart();
    try {
      if (!reconnecting) {
        if (isSSR) {
          // Seed explicit authored state. A streamed activation (forceSSR)
          // supplies its boundary-local state directly; ordinary hydration
          // defaults to the global `window.__webui.state` handoff. Passing a
          // boundary's state as-is (even when undefined) keeps a stateless
          // streamed boundary from falling back to a later boundary's global
          // state.
          if (this.$shouldApplySSRBootstrapState()) {
            this.$applySSRState(
              hasBoundaryState ? ssrState : window.__webui?.state,
            );
          }
        }
        this.$applyPendingParentState(isSSR);
      }

      if (isSSR) {
        this.$root = this.$hydrate(root, meta, getTemplateDom(meta));

      } else {
        clientRoot = this.$createStagingRoot(meta);
        this.$root = this.$wire(clientRoot, meta);
      }

      this.$meta = meta;
      this.$hydrated = true;
      this.$ready = true;
      this.$syncAuthoredAttributes();
      if (isSSR && this.$deferredWrites) this.$replayDeferredWrites();
      if (isSSR && reconnecting) {
        // Retained DOM is client-owned after the first mount. Reconcile roots
        // that are still available while preserving trusted values for any
        // template-only state the client never received.
        this.$updateBindings(
          this.$root.texts,
          this.$root.attrs,
          this.$root.conds,
          this.$root.repeats,
          true,
        );
      }

      // SSR only: warn when a pre-ready write left an observable disagreeing
      // with the server-rendered DOM. Client-created components have no SSR
      // content to diverge from. `DEV` gates the call so production bundles
      // (`--define:__WEBUI_DEV__=false`) drop it entirely.
      if (isSSR && DEV) this.$checkHydrationMismatch();
      else this.$preReadyWrites = null;

      // Client-created components: flush current attr/observable values
      // into the freshly-wired template DOM. Call $updateInstance directly
      // to avoid the $update() path-index build — it will be lazy-built
      // on the first reactive change instead.
      if (!isSSR && clientRoot) {
        this.$updateInstance(this.$root);
        const hasStructuralBindings =
          this.$root.repeats.length !== 0 || this.$root.conds.length !== 0;
        if (hasStructuralBindings) {
          this.$root.nodes = childNodesArray(clientRoot);
        }
        let deferredStyles = false;
        if (templateMayContainLinkStyles(meta)) {
          const stagingRoot = clientRoot;
          const mountedInstance = this.$root;
          deferredStyles = installTemplateLinkStyles(
            this,
            meta,
            this.shadowRoot,
            clientRoot,
            this.$root.attrs,
            {
              hasAuthorHydrationLifecycle:
                this.hydratedCallback !==
                TemplateElement.prototype.hydratedCallback,
              beforeAppend: () => {
                if (this.$root !== mountedInstance || !this.$hydrated) return;
                this.$updateInstance(mountedInstance);
                if (hasStructuralBindings) {
                  mountedInstance.nodes = childNodesArray(stagingRoot);
                }
              },
              afterAppend: () => {
                if (this.$root !== mountedInstance || !this.$hydrated) return;
                this.$replaceInstanceContainer(
                  mountedInstance,
                  clientRoot,
                  root as ParentNode & Node,
                );
                mountedInstance.container = root as ParentNode & Node;
                this.$deferredClientMount = false;
                this.$ready = true;
                this.$finishHydration();
              },
            },
          );
        }
        if (deferredStyles) {
          this.$deferredClientMount = true;
          this.$ready = false;
          deferredHydrationFinish = true;
        }
        if (!deferredStyles && hasStructuralBindings) {
          this.$replaceInstanceContainer(
            this.$root,
            clientRoot,
            root as ParentNode & Node,
          );
        }
        if (!deferredStyles) this.$appendStagedChildren(root, clientRoot);
        if (!deferredStyles) this.$root.container = root as ParentNode & Node;
      }
      if (!deferredHydrationFinish) this.$finishHydration();
    } finally {
      hydrationEnd();
    }
  }

  private $installStyles(meta: TemplateMeta): void {
    const wantShadow = !!this.shadowRoot || !!meta.sd;
    const containingRoot = this.getRootNode();
    const styleTarget = wantShadow
      ? this.shadowRoot!
      : containingRoot.nodeType === 11 && 'host' in containingRoot
        ? containingRoot as ShadowRoot
        : this.ownerDocument;
    claimSsrComponentStyles(this, styleTarget);
    const installation = installComponentStyles(this.localName, styleTarget);
    if (installation) {
      void installation.catch((error) => {
        console.error(error);
      });
    }
  }

  disconnectedCallback(): void {
    this.$detachDeferredAncestor();
    // Schedule teardown on microtask — if the element is re-connected
    // before then (e.g. repeat reconciliation), skip the cleanup.
    if (this.$root) {
      queueMicrotask(() => {
        if (!this.isConnected) this.$destroy();
      });
    }
  }

  /**
   * Permanently destroy this component's own bindings and DOM references.
   * Each component is responsible for its own cleanup — child WebUI
   * elements handle theirs via their own `disconnectedCallback`.
   */
  $destroy(): void {
    const shadowRoot = this.shadowRoot;
    if (shadowRoot && cancelTemplateLinkStyleMount(shadowRoot)) {
      this.$resetClientShadow = true;
    }
    if (!this.$root) {
      this.$detachDeferredAncestor();
      this.$abandonDeferredDescendants();
      this.$deferredSSR = false;
      this.$deferredByAncestor = undefined;
      this.$ancestorBoundaryState = undefined;
      this.$hasAncestorBoundaryState = undefined;
      this.$ready = false;
      this.$hasUnknownScopes = false;
      if (this.$deferredWrites) this.$deferredWrites = undefined;
      return;
    }
    this.$disposeInstance(this.$root, false);
    this.$root = null;
    this.$pathIndex = undefined;
    this.$wildcardBindings = undefined;
    this.$dirtyPaths = null;
    this.$pendingFlush = false;
    this.$preReadyWrites = null;
    this.$hydrated = false;
    this.$deferredClientMount = false;
    this.$ready = false;
    this.$hasUnknownScopes = false;
  }

  private $clearRepeatBinding(repeat: RepeatBinding): void {
    repeat.instances.length = 0;
    repeat.container = null;
    repeat.start = null;
    repeat.end = null;
    const state = repeat.keyState;
    if (state) {
      state.keys.length = 0;
      state.nextKeys.length = 0;
      state.map.clear();
    }
  }

  private $clearInstance(instance: TemplateInstance): void {
    instance.scope = undefined;
    instance.parent = undefined;
    instance.container = null;
    instance.nodes.length = 0;
    instance.texts.length = 0;
    instance.attrs.length = 0;
    instance.conds.length = 0;
    instance.repeats.length = 0;
  }

  attributeChangedCallback(
    name: string,
    oldValue: string | null,
    newValue: string | null,
  ): void {
    if (Object.is(oldValue, newValue)) return;
    const wasDeferred = this.$deferredSSR;
    this.$beforeExternalStateWrite();
    const property = this.$templateRootForAttribute(name);
    let changed = false;
    if (property && this.$usesTemplateState(property)) {
      if (wasDeferred) this.$recordDeferredWrite(property);
      changed = this.$setTemplateState(property, newValue);
    }
    this.$afterExternalStateWrite(changed);
  }

  /** Populate component state from server or router state.
   *
   * Decorated properties are set through their reactive setters. Template-only
   * bindings are stored internally so app code does not need public
   * `@observable` fields just to receive server state.
   */
  setState(state: Record<string, unknown>): void {
    const wasDeferred = this.$deferredSSR;
    this.$beforeExternalStateWrite();
    const keys = Object.keys(state);
    let owned = false;
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      const keyOwned = this.$setStateKey(key, state[key]);
      if (keyOwned && wasDeferred) this.$recordDeferredWrite(key);
      owned = keyOwned || owned;
    }
    this.$afterExternalStateWrite(owned);
    this.$flushUpdates();
  }

  protected $observableNames(): Set<string> {
    return EMPTY_SET;
  }

  /** Prepare a dormant template host before an external state write. */
  protected $beforeExternalStateWrite(): void {
  }

  /**
   * Finish preparing a dormant template host after an external state write.
   *
   * `applied` is true when the write reached this component's state — an owned
   * key (`setState` / parent binding) or a changed template attribute.
   */
  protected $afterExternalStateWrite(_applied: boolean): void {
  }

  /**
   * Decide whether an SSR instance should remain dormant until client use.
   *
   * Authored components defer only when their compiler-owned `data-ws` marker
   * is present; `connectedCallback()` handles that case before mounting.
   * Keeping the default false prevents unrelated client-created light-DOM
   * components on a streaming page from becoming permanently dormant.
   * Compiler-owned static hosts override this to retain their existing
   * dormant-until-state-write behavior.
   */
  protected $shouldDeferSSRHydration(_meta?: TemplateMeta): boolean {
    return false;
  }

  /** Respond after an SSR instance enters or reconnects in deferred mode. */
  protected $didDeferSSRHydration(): void {
  }

  /** Retain boundary-local state when a streamed child remains visibility-deferred. */
  protected $didDeferStreamedSSRHydration(
    _state: Record<string, unknown> | undefined,
  ): void {
    this.$didDeferSSRHydration();
  }

  /** Return this instance's compiled component metadata when available. */
  protected $currentTemplateMetadata(): TemplateMeta | undefined {
    return this.$meta ?? this.$templateMeta();
  }

  private $nearestHydrationBarrier(): Element | undefined {
    let current: Element = this;
    while (true) {
      let parent: Element | null =
        current.assignedSlot ?? current.parentElement;
      if (!parent) {
        const getRootNode = (
          current as Element & { getRootNode?: () => Node }
        ).getRootNode;
        const root = typeof getRootNode === 'function'
          ? getRootNode.call(current)
          : null;
        parent = typeof ShadowRoot !== 'undefined' &&
          root instanceof ShadowRoot
          ? root.host
          : null;
      }
      if (!parent) return undefined;
      if (parent instanceof TemplateElement) {
        const parentMeta = parent.$meta ?? parent.$templateMeta();
        if (parentMeta?.th) {
          current = parent;
          continue;
        }
        if (parent.$deferredSSR || !parent.$hydrated) return parent;
      } else {
        const parentMeta = getTemplate(parent.tagName.toLowerCase());
        if (parentMeta && !parentMeta.th) return parent;
      }
      current = parent;
    }
  }

  private $registerWithHydrationBarrier(ancestor: Element): void {
    if (ancestor instanceof TemplateElement) {
      ancestor.$registerDeferredDescendant(this);
      return;
    }

    this.$detachDeferredAncestor();
    this.$pendingAncestor = ancestor;
    const descendants = pendingAncestorDescendants.get(ancestor);
    if (descendants) {
      descendants.push(this);
    } else {
      pendingAncestorDescendants.set(ancestor, [this]);
    }
  }

  private $adoptPendingDescendants(): void {
    const descendants = pendingAncestorDescendants.get(this);
    if (!descendants) return;
    pendingAncestorDescendants.delete(this);
    for (let i = 0; i < descendants.length; i++) {
      const descendant = descendants[i];
      if (descendant.$pendingAncestor !== this) continue;
      descendant.$pendingAncestor = undefined;
      if (descendant.isConnected) this.$registerDeferredDescendant(descendant);
    }
  }

  private $registerDeferredDescendant(descendant: TemplateElement): void {
    if (descendant.$deferredAncestor === this) return;
    descendant.$detachDeferredAncestor();
    descendant.$deferredAncestor = this;
    (this.$deferredDescendants ??= []).push(descendant);
  }

  private $detachDeferredAncestor(): void {
    const pendingAncestor = this.$pendingAncestor;
    if (pendingAncestor) {
      this.$pendingAncestor = undefined;
      const pending = pendingAncestorDescendants.get(pendingAncestor);
      if (pending) {
        const pendingIndex = pending.indexOf(this);
        if (pendingIndex >= 0) pending.splice(pendingIndex, 1);
        if (pending.length === 0) {
          pendingAncestorDescendants.delete(pendingAncestor);
        }
      }
    }

    const ancestor = this.$deferredAncestor;
    if (!ancestor) return;
    this.$deferredAncestor = undefined;
    const descendants = ancestor.$deferredDescendants;
    if (!descendants) return;
    const index = descendants.indexOf(this);
    if (index >= 0) descendants.splice(index, 1);
    if (descendants.length === 0) ancestor.$deferredDescendants = undefined;
  }

  private $releaseDeferredDescendants(): void {
    const descendants = this.$deferredDescendants;
    if (!descendants) return;
    this.$deferredDescendants = undefined;
    const queue = TemplateElement.$ancestorReleaseQueue;
    for (let i = 0; i < descendants.length; i++) {
      const descendant = descendants[i];
      if (descendant.$deferredAncestor !== this) continue;
      descendant.$deferredAncestor = undefined;
      queue.push(descendant);
    }
    if (TemplateElement.$ancestorReleaseActive) return;

    TemplateElement.$ancestorReleaseActive = true;
    let errors: unknown[] | undefined;
    try {
      while (TemplateElement.$ancestorReleaseIndex < queue.length) {
        const descendant = queue[TemplateElement.$ancestorReleaseIndex];
        TemplateElement.$ancestorReleaseIndex++;
        try {
          descendant.$releaseAncestorBarrier();
        } catch (error) {
          (errors ??= []).push(error);
        }
      }
    } finally {
      queue.length = 0;
      TemplateElement.$ancestorReleaseIndex = 0;
      TemplateElement.$ancestorReleaseActive = false;
    }
    if (errors) {
      if (errors.length === 1) throw errors[0];
      throw new AggregateError(
        errors,
        'multiple deferred descendants failed to hydrate',
      );
    }
  }

  private $releaseAncestorBarrier(): void {
    if (!this.$deferredByAncestor || !this.isConnected) return;
    this.$deferredByAncestor = undefined;
    const hasBoundaryState = this.$hasAncestorBoundaryState === true;
    const boundaryState = this.$ancestorBoundaryState;
    this.$hasAncestorBoundaryState = undefined;
    this.$ancestorBoundaryState = undefined;
    const meta = this.$meta;
    if (meta && this.$shouldDeferSSRHydration(meta)) {
      if (hasBoundaryState) {
        this.$didDeferStreamedSSRHydration(boundaryState);
      } else {
        this.$didDeferSSRHydration();
      }
      return;
    }
    if (hasBoundaryState) {
      this.$activateDeferredSSRFromBoundary(boundaryState);
    } else {
      this.$activateDeferredSSR();
    }
  }

  private $abandonDeferredDescendants(): void {
    const descendants = this.$deferredDescendants;
    if (!descendants) return;
    this.$deferredDescendants = undefined;
    for (let i = 0; i < descendants.length; i++) {
      if (descendants[i].$deferredAncestor === this) {
        descendants[i].$deferredAncestor = undefined;
      }
    }
  }

  /**
   * Seed ordinary bootstrap state before a lazy root releases the initial
   * `window.__webui.state` handoff. Values stay in component-local fields, so
   * delayed activation does not retain the page-wide bootstrap object.
   */
  protected $primeSSRStateForDeferral(): void {
    if (this.$shouldApplySSRBootstrapState()) {
      this.$applySSRState(window.__webui?.state);
    }
    this.$applyPendingParentState(true);
  }

  /**
   * Decide whether a streamed boundary commit should activate this instance.
   * Authored/base components activate immediately; compiler-owned static
   * hosts (`static-host.ts`) opt out to keep their existing
   * dormant-until-state-write contract.
   */
  protected $shouldActivateOnBoundaryCommit(): boolean {
    return true;
  }

  /** Activate a previously deferred SSR instance. `state` is the boundary-local
   *  SSR state supplied by the streaming coordinator; ordinary (non-streaming)
   *  activations omit it and fall back to the global handoff inside `$mount`. */
  protected $activateDeferredSSR(state?: Record<string, unknown>): void {
    this.$activateDeferredSSRWithState(
      state,
      this.$activatingDeferredSSR,
    );
  }

  /**
   * Activate deferred SSR using retained boundary-local state after the
   * streaming coordinator has released its boundary record.
   */
  protected $activateDeferredSSRFromBoundary(
    state?: Record<string, unknown>,
  ): void {
    this.$activateDeferredSSRWithState(state, true);
  }

  private $activateDeferredSSRWithState(
    state: Record<string, unknown> | undefined,
    hasBoundaryState: boolean,
  ): void {
    if (!this.$deferredSSR) return;
    // Both activation owners establish metadata before calling this hook:
    // static hosts retain it in `$mount()`, and streamed roots validate/cache it
    // in the coordinator hook.
    const meta = this.$meta;
    if (!meta) return;
    this.$deferredSSR = false;
    this.$guardUnknownState = true;
    this.$ready = false;
    this.$preReadyWrites = null;
    const wasActivating = this.$activatingDeferredSSR;
    this.$activatingDeferredSSR = true;
    try {
      this.$mount(meta, true, state, hasBoundaryState);
    } finally {
      this.$activatingDeferredSSR = wasActivating;
      // The streamed-host `data-ws` marker is NOT dropped here: the streaming
      // coordinator (`streaming.ts` `invokeActivationHook`) owns successful-path
      // removal in its own `finally`, so every committed root — including
      // no-hook and opt-out (static host) roots — is stripped uniformly, even
      // if this activation threw. Removing it here too would be redundant.
    }
  }

  /** Decide whether this component consumes the global SSR bootstrap state. */
  protected $shouldApplySSRBootstrapState(): boolean {
    return true;
  }

  /** Decide whether a decorated property should be initialized from SSR state. */
  protected $shouldApplySSRState(_key: string): boolean {
    return true;
  }

  /** Sync authored attribute-backed properties after mount. */
  protected $syncAuthoredAttributes(): void {
  }

  /**
   * Runs synchronously once after this instance first hydrates or mounts.
   *
   * Unlike `connectedCallback()`, this hook does not run again on reconnect and
   * is not called until a deferred streamed or static host actually activates.
   */
  protected hydratedCallback(): void {
  }

  private $notifyHydrated(): void {
    if (this.$hasMounted) return;
    // Latch before author code so an exception can never turn reconnect into a
    // retry of a lifecycle that has already been entered.
    this.$hasMounted = true;
    this.hydratedCallback();
  }

  private $finishHydration(): void {
    let callbackFailed = false;
    let callbackError: unknown;
    try {
      this.$notifyHydrated();
    } catch (error) {
      callbackFailed = true;
      callbackError = error;
    }

    try {
      this.$releaseDeferredDescendants();
    } catch (releaseError) {
      if (callbackFailed) {
        throw new AggregateError(
          [callbackError, releaseError],
          'component callback and deferred descendant hydration failed',
        );
      }
      throw releaseError;
    }
    if (callbackFailed) throw callbackError;
  }

  /** Decide whether hidden template state should be initialized from SSR state. */
  protected $shouldApplyTemplateStateFromSSR(_key: string): boolean {
    return true;
  }

  /** Write hidden template state and update bindings that read this root. */
  protected $setTemplateState(key: string, value: unknown): boolean {
    const changed = this.$writeTemplateState(key, value);
    if (changed) {
      this.$update(key);
    }
    return changed;
  }

  private $writeTemplateState(key: string, value: unknown): boolean {
    if (!this.$templateState) {
      this.$templateState = Object.create(null) as Record<string, unknown>;
    }
    if (
      Object.prototype.hasOwnProperty.call(this.$templateState, key) &&
      Object.is(this.$templateState[key], value)
    ) return false;
    this.$templateState[key] = value;
    return true;
  }

  private $templateMeta(): TemplateMeta | undefined {
    if (this.$meta) return this.$meta;
    const fromCtor = templateMetaByCtor.get(this.constructor as Function);
    if (fromCtor) return fromCtor;
    const tagName = this.tagName;
    return tagName ? getTemplate(tagName.toLowerCase()) : undefined;
  }

  private $templateRootForAttribute(name: string): string | undefined {
    const meta = this.$templateMeta();
    return meta ? templateRootForAttribute(meta, name) : undefined;
  }

  private $usesTemplateState(key: string): boolean {
    const meta = this.$templateMeta();
    return !!meta && templateHasRoot(meta, key) && !hasAuthoredMember(this, key);
  }

  /**
   * Route one external state key to an authored observable or hidden template
   * state, returning whether this component *owns* the key.
   *
   * The return is "owned", NOT "value changed": it stays `true` for an owned
   * key even when the write is a no-op. `$patchAttr`'s complex-binding fallback
   * relies on this — a `false` return makes the parent assign a plain DOM
   * property (`el[name] = v`) that would shadow owned template state. Do not
   * "optimize" this to real change-detection.
   */
  private $setStateKey(key: string, value: unknown): boolean {
    if (this.$observableNames().has(key)) {
      (this as Record<string, unknown>)[key] = value;
      return true;
    }
    if (this.$usesTemplateState(key)) {
      this.$setTemplateState(key, value);
      return true;
    }
    return false;
  }

  /**
   * Apply SSR state to this instance's `@observable`/`@attr` and template
   * backing fields.
   *
   * `state` is supplied by the caller: ordinary hydration passes
   * `window.__webui.state` (the consolidated SSR bootstrap block), while a
   * streamed activation passes its boundary-local state directly so a late
   * activation sees the state that was live when its *own* boundary committed
   * rather than whatever the global handoff currently holds. The build-time
   * hydration keys contain only explicit `@observable`/`@attr` properties;
   * template-only roots remain absent because their initial values are already
   * represented by the trusted SSR DOM.
   *
   * Writes directly to the backing field (`_prop`) to avoid triggering
   * reactive updates before bindings are wired.
   */
  private $applySSRState(state: Record<string, unknown> | undefined): void {
    if (!state || typeof state !== 'object') return;
    const observableNames = this.$observableNames();
    const deferredWrites = this.$deferredWrites;
    const keys = Object.keys(state);
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (deferredWrites?.has(key)) continue;
      if (observableNames.has(key)) {
        if (!this.$shouldApplySSRState(key)) continue;
        // Write to backing field directly — no reactive update yet
        (this as Record<string, unknown>)[`_${key}`] = state[key];
      } else if (this.$usesTemplateState(key) && this.$shouldApplyTemplateStateFromSSR(key)) {
        this.$writeTemplateState(key, state[key]);
      }
    }
  }

  /**
   * Apply instance-local state supplied by an SSR parent before this child
   * walks its own bindings. Parent values override page-wide bootstrap keys.
   */
  private $applyPendingParentState(replayAfterHydration: boolean): void {
    const pending = pendingParentStateByElement.get(this);
    if (!pending) return;
    pendingParentStateByElement.delete(this);

    const state = pending.values;
    const observableNames = this.$observableNames();
    const keys = Object.keys(state);
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (observableNames.has(key)) {
        (this as Record<string, unknown>)[`_${key}`] = state[key];
      } else if (this.$usesTemplateState(key)) {
        this.$writeTemplateState(key, state[key]);
      } else {
        (this as Record<string, unknown>)[key] = state[key];
      }
    }
    const replay = pending.replay;
    if (replayAfterHydration && replay) {
      const deferredWrites = this.$deferredWrites;
      if (deferredWrites) {
        for (const key of replay) deferredWrites.add(key);
      } else {
        this.$deferredWrites = replay;
      }
    }
  }

  /** Reactive update — called by @observable/@attr setters. */
  $update(path?: string): void {
    if (!this.$ready || !this.$root) {
      if (this.$deferredClientMount) return;
      if (path && this.$deferredSSR) this.$recordDeferredWrite(path);
      // A reactive write arrived while connected but before hydration
      // completed. `$update` cannot touch the DOM yet, so record the path and
      // check it against the SSR DOM once hydrated (see #379). `DEV` gates the
      // recording so production bundles never allocate the tracking Set.
      if (DEV && path && this.isConnected) this.$recordPreReadyWrite(path);
      return;
    }

    // Lazy-build path index on first update (deferred from hydration)
    if (!this.$pathIndex) this.$buildPathIndex();

    if (path && this.$pathIndex) {
      const entry = this.$pathIndex.get(path);
      if (entry) {
        // Batch path-specific updates via microtask coalescing.
        if (!this.$dirtyPaths) this.$dirtyPaths = new Set();
        this.$dirtyPaths.add(path);
        if (!this.$pendingFlush) {
          this.$pendingFlush = true;
          queueMicrotask(() => this.$flush());
        }
        return;
      }
    }

    // Full immediate update (initial mount, reconnect, or unknown path).
    this.$dirtyPaths = null;
    this.$updateInstance(this.$root);
  }

  private $recordDeferredWrite(path: string): void {
    const dot = path.indexOf('.');
    const root = dot === -1 ? path : path.slice(0, dot);
    (this.$deferredWrites ??= new Set()).add(root);
  }

  private $replayDeferredWrites(): void {
    const writes = this.$deferredWrites;
    if (!writes) return;
    this.$deferredWrites = undefined;
    if (this.$dirtyPaths) {
      for (const path of writes) this.$dirtyPaths.add(path);
    } else {
      this.$dirtyPaths = writes;
    }
    this.$pendingFlush = true;
    this.$flush(true);
  }

  /** Synchronously flush all queued path updates. Call this when you need
   *  the DOM to reflect pending property changes immediately. */
  $flushUpdates(): void {
    if (this.$pendingFlush) this.$flush();
  }

  /** Flush all queued path updates. Handles re-entrant setter calls. */
  private $flush(
    requireKnownState = this.$guardUnknownState === true,
  ): void {
    if (!this.$ready || !this.$root) {
      this.$dirtyPaths = null;
      this.$pendingFlush = false;
      return;
    }
    if (!this.$pathIndex) this.$buildPathIndex();
    if (!this.$pathIndex) return;

    while (this.$dirtyPaths && this.$dirtyPaths.size > 0) {
      // Snapshot and clear so re-entrant setters get a fresh set.
      const dirty = this.$dirtyPaths;
      this.$dirtyPaths = null;

      for (const path of dirty) {
        if (!this.$pathIndex) this.$buildPathIndex();
        const entry = this.$pathIndex?.get(path);
        if (entry) {
          this.$updateBindings(
            entry.texts,
            entry.attrs,
            entry.conds,
            entry.repeats,
            requireKnownState,
          );
        }
      }
      // Update wildcard bindings once per flush (not per dirty path)
      if (!this.$pathIndex) this.$buildPathIndex();
      if (this.$wildcardBindings) {
        const wc = this.$wildcardBindings;
        this.$updateBindings(
          wc.texts,
          wc.attrs,
          wc.conds,
          wc.repeats,
          requireKnownState,
        );
      }
      if (!this.$pathIndex) this.$buildPathIndex();
    }

    this.$pendingFlush = false;
  }

  // ── Hydration mismatch diagnostic (#379) ──────────────────────
  // A reactive write that runs while the element is connected but before
  // hydration finishes (constructor, `@observable` field initializer, or
  // before `super.connectedCallback()`) is dropped by `$update`'s pre-ready
  // guard. Record such writes and, once hydrated, report any that disagree
  // with the trusted SSR DOM. The read-only comparison lives in
  // `hydration-mismatch.ts` (see that module for the full rationale); this
  // class only records the writes and supplies a resolver context.
  //
  // Note: `$applySSRState` (in `$mount`) has already overwritten the backing
  // field of every observable present in the SSR state before this runs, so
  // those reconcile to the server value and cannot disagree. In practice the
  // diagnostic only fires for observables omitted from the SSR state.
  //
  // Production stripping: the comparators and message string live in
  // `hydration-mismatch.ts`, reached only through the dynamic `import()` in
  // `$checkHydrationMismatch`. When a bundler folds `__WEBUI_DEV__` to `false`,
  // `DEV` becomes a constant, the `if (!DEV) return` empties this method, and
  // the now-dead `import()` is DCE'd — orphaning the diagnostic chunk so the
  // bundler drops it. The `if (!DEV) return` is load-bearing: class methods are
  // never tree-shaken, so without it the method body (and its `import()`) would
  // survive. See the `__WEBUI_DEV__` note near the top of this file.

  private $recordPreReadyWrite(path: string): void {
    if (!this.$preReadyWrites) this.$preReadyWrites = new Set();
    this.$preReadyWrites.add(path);
  }

  private $checkHydrationMismatch(): void {
    if (!DEV) return;
    const writes = this.$preReadyWrites;
    this.$preReadyWrites = null;
    if (!writes || writes.size === 0 || !this.$root) return;
    if (!this.$pathIndex) this.$buildPathIndex();
    const index = this.$pathIndex;
    if (!index) return;
    const ctx: MismatchContext = {
      resolver: this.$resolver,
      resolveParts: (parts, scope) => this.$resolveParts(parts, scope),
      resolveValue: (path, scope) => this.$resolveValue(path, scope),
    };
    const tag = this.tagName.toLowerCase();
    // Defer the read-only comparison to the dynamically-imported diagnostic
    // module. `writes`, `index`, and `ctx` are captured synchronously here; the
    // SSR DOM they compare against does not change between `$ready` and the
    // microtask on which the import resolves, so the result is unaffected by the
    // deferral. In production `DEV` folds to `false`, so this method's body — and
    // therefore this sole `import()` — is eliminated, dropping the diagnostic
    // module from the bundle (see the `__WEBUI_DEV__` note near the top).
    void import('./hydration-mismatch.js').then((m) =>
      m.reportHydrationMismatch(tag, writes, index, ctx),
    );
  }

  // ── DOM resolution: client-created path ───────────────────────
  // Compiled paths are childNode indices in meta.h parsed by the browser.
  // For client-created components the DOM matches meta.h exactly.

  // ── Template parsing ──────────────────────────────────────────

  private $parseTemplate(meta: TemplateBlockMeta): DocumentFragment {
    return cloneTemplateContent(meta);
  }

  private $createStagingRoot(meta: TemplateBlockMeta): HTMLElement {
    const wrapper = document.createElement('div');
    const fragment = this.$parseTemplate(meta);
    wrapper.appendChild(fragment);
    customElements.upgrade(wrapper);
    return wrapper;
  }

  private $appendStagedChildren(root: Node, stagingRoot: Node): void {
    const first = stagingRoot.firstChild;
    if (!first) return;
    if (!first.nextSibling) {
      root.appendChild(first);
      return;
    }
    const fragment = document.createDocumentFragment();
    while (stagingRoot.firstChild) {
      fragment.appendChild(stagingRoot.firstChild);
    }
    root.appendChild(fragment);
  }

  private $replaceInstanceContainer(
    instance: TemplateInstance,
    previous: (ParentNode & Node) | null,
    next: (ParentNode & Node) | null,
  ): void {
    if (previous === next) return;
    const stack: TemplateInstance[] = [instance];
    while (stack.length > 0) {
      const current = stack.pop();
      if (!current) continue;
      if (current.container === previous) current.container = next;
      for (let i = 0; i < current.conds.length; i++) {
        const child = current.conds[i].instance;
        if (child) stack.push(child);
      }
      for (let i = 0; i < current.repeats.length; i++) {
        const repeat = current.repeats[i];
        if (repeat.container === previous) repeat.container = next;
        const children = repeat.instances;
        for (let j = 0; j < children.length; j++) stack.push(children[j]);
      }
    }
  }

  // ═══════════════════════════════════════════════════════════════
  //  Client-created wiring — exact childNode index resolution
  // ═══════════════════════════════════════════════════════════════

  private $wire(root: Node, meta: TemplateBlockMeta, scope?: ScopeFrame): TemplateInstance {
    const instance: TemplateInstance = {
      scope, container: root as ParentNode & Node, nodes: childNodesArray(root),
      texts: [], attrs: [], conds: [], repeats: [],
    };

    // Resolve ALL slot reference nodes BEFORE inserting any anchors.
    // Inserting comment anchors shifts childNode indices, so we must
    // capture target positions from the untouched DOM first.
    //
    // Cloned template DOM matches `h` exactly, so numbering its elements in
    // pre-order reproduces the indices the compiler assigned.
    const elements = collectTemplateElements(root);

    // Pre-resolve text binding slots
    const textRefs = new Array<{ parent: Node; ref: Node | null; parts: CompiledAttrPart[]; raw?: boolean }>(meta.tx?.length ?? 0);
    let textRefCount = 0;
    if (meta.tx) {
      for (let i = 0; i < meta.tx.length; i++) {
        const entry = meta.tx[i];
        const [slot, parts] = entry;
        const raw = entry[2] === 1;
        const [parentIndex, beforeIndex] = slot;
        const parent = elements[parentIndex];
        if (!parent || (parent.nodeType !== 1 && parent.nodeType !== 11)) continue;
        textRefs[textRefCount] = { parent, ref: parent.childNodes[beforeIndex] || null, parts, raw };
        textRefCount += 1;
      }
    }

    // Pre-resolve conditional slots
    type CondRef = { parent: Node; ref: Node | null; condition: CompiledCondition; blockIndex: number };
    const condRefs = new Array<CondRef>(meta.c?.length ?? 0);
    let condRefCount = 0;
    if (meta.c) {
      for (let i = 0; i < meta.c.length; i++) {
        const [condition, blockIndex, slotMeta] = meta.c[i];
        const [parentIndex, beforeIndex] = slotMeta;
        const parent = elements[parentIndex];
        if (!parent || (parent.nodeType !== 1 && parent.nodeType !== 11)) continue;
        condRefs[condRefCount] = { parent, ref: parent.childNodes[beforeIndex] || null, condition: condition as CompiledCondition, blockIndex };
        condRefCount += 1;
      }
    }

    // Pre-resolve repeat slots
    type RepRef = {
      parent: Node;
      ref: Node | null;
      collection: string;
      itemVar: string;
      blockIndex: number;
      keyPath?: string;
    };
    const repRefs = new Array<RepRef>(meta.r?.length ?? 0);
    let repRefCount = 0;
    if (meta.r) {
      for (let i = 0; i < meta.r.length; i++) {
        const [collection, itemVar, blockIndex, slotMeta, keyPath] = meta.r[i];
        const [parentIndex, beforeIndex] = slotMeta;
        const parent = elements[parentIndex];
        if (!parent || (parent.nodeType !== 1 && parent.nodeType !== 11)) continue;
        repRefs[repRefCount] = {
          parent,
          ref: parent.childNodes[beforeIndex] || null,
          collection,
          itemVar,
          blockIndex,
          keyPath,
        };
        repRefCount += 1;
      }
    }

    // Attribute bindings (no DOM mutation — safe to resolve inline)
    this.$wireAttrs(instance, meta, scope, (i) => elements[i] ?? null);

    // Events + refs — resolve BEFORE anchors shift childNode indices.
    // Events target element nodes (not text/comment positions), but anchor
    // insertions still shift childNode indices for sibling elements.
    this.$finalize(instance, root, meta, (_r, i) => elements[i] ?? null, scope);

    // Now insert anchors using pre-resolved references

    // Text bindings
    for (let i = 0; i < textRefCount; i++) {
      const t = textRefs[i];
      const anchor = document.createComment('');
      t.parent.insertBefore(anchor, t.ref);
      if (t.raw) {
        // Raw binding: create a container span for innerHTML updates
        const container = document.createElement('span');
        t.parent.insertBefore(container, anchor);
        const textNode = document.createTextNode('');
        instance.texts.push({ node: textNode, parts: t.parts, scope, raw: true, rawParent: container });
      } else {
        const textNode = document.createTextNode('');
        t.parent.insertBefore(textNode, anchor);
        instance.texts.push({ node: textNode, parts: t.parts, scope });
      }
    }

    // Conditional bindings
    for (let i = 0; i < condRefCount; i++) {
      const c = condRefs[i];
      const anchor = document.createComment('');
      c.parent.insertBefore(anchor, c.ref);
      instance.conds.push({
        condition: c.condition,
        blockIndex: c.blockIndex,
        anchor,
        scope,
        owner: instance,
        instance: null,
      });
    }

    // Repeat bindings
    for (let i = 0; i < repRefCount; i++) {
      const r = repRefs[i];
      const anchor = document.createComment('');
      r.parent.insertBefore(anchor, r.ref);
      const binding: RepeatBinding = {
        markerId: i, collection: r.collection, itemVar: r.itemVar, blockIndex: r.blockIndex,
        container: r.parent as ParentNode & Node, start: anchor, end: null,
        scope, owner: instance, instances: [],
      };
      if (r.keyPath !== undefined) {
        binding.keyState = createRepeatKeyState(r.keyPath);
      }
      instance.repeats.push(binding);
    }

    // Evaluate conditionals and repeats inline so blocks are created
    // immediately — no deferred $update() flush needed.
    for (let i = 0; i < instance.conds.length; i++) this.$toggleCond(instance.conds[i]);

    return instance;
  }

  // ═══════════════════════════════════════════════════════════════
  //  SSR hydration — marker-based in-place DOM matching
  // ═══════════════════════════════════════════════════════════════

  /**
   * Hydrate SSR-rendered DOM against compiled template metadata.
   *
   * When pathStart=0 (default): ssrRoot is a container with children
   * (top-level component hydration).
   *
   * When pathStart=1: ssrRoot is a block element itself (repeat item
   * in-place hydration). The leading [0] wrapper segment is skipped
   * so compiled paths resolve directly against the element.
   */
  private $hydrate(
    ssrRoot: Node,
    meta: TemplateBlockMeta,
    tplDom: Element,
    scope?: ScopeFrame,
    pathStart = 0,
  ): TemplateInstance {
    const instance: TemplateInstance = {
      scope,
      container: (pathStart > 0 ? ssrRoot.parentNode : ssrRoot) as (ParentNode & Node) | null,
      nodes: pathStart > 0 ? [ssrRoot] : childNodesArray(ssrRoot),
      texts: [], attrs: [], conds: [], repeats: [],
    };

    // Collect SSR markers for deferred removal.  Closing markers
    // (<!--/wc-->, <!--/wr-->) and item markers (<!--wi-->) must stay in
    // the DOM throughout the entire hydration pass so that the index walk
    // and $findSSRText can correctly skip structural block content when
    // counting element/text ordinals.  All collected markers are removed
    // in a single cleanup pass after $finalize() (events + refs).
    //
    // Hydration order:  text → attrs → conditionals → repeats → events
    // Every phase reads the index or the markers, so both must survive to the end.
    const staleMarkers: Node[] = [];

    // Resolve the whole subtree up front.  Every binding used to walk down
    // from the root on its own, rescanning each parent's children, which made
    // hydration cost O(bindings × width).  One pre-order pass pairs template
    // elements with their SSR counterparts and collects the block markers in
    // document order, so each binding below is an O(1) lookup.
    //
    // Built before any mutation: the phases below insert text nodes and
    // anchors, but never add or permanently remove elements outside a block
    // range, so the element pairing stays valid for the whole pass.
    const ssrIndex = buildSSRIndex(
      tplDom,
      ssrRoot,
      meta.c !== undefined || meta.r !== undefined,
      pathStart > 0,
    );
    const ssrElements = ssrIndex.elements;
    const tplElements = getTemplateElements(tplDom);

    // Text bindings — find existing text nodes rendered by the server
    if (meta.tx) {
      for (let i = 0; i < meta.tx.length; i++) {
        const entry = meta.tx[i];
        const [slot, parts] = entry;
        const raw = entry[2] === 1;
        const [parentIndex, beforeIndex] = slot;
        const ssrParent = ssrElements[parentIndex];
        if (!ssrParent) continue;
        const tplParent = tplElements[parentIndex];
        if (!tplParent) continue;
        if (raw) {
          const rawParent = ssrParent as Element;
          const textNode = document.createTextNode('');
          instance.texts.push({ node: textNode, parts, scope, raw: true, rawParent });
        } else {
          let textNode = this.$findSSRText(ssrParent, tplParent, beforeIndex);
          if (!textNode) {
            textNode = document.createTextNode('');
            const insertRef = this.$findSSRSlotRef(ssrParent, tplParent, beforeIndex);
            ssrParent.insertBefore(textNode, insertRef);
          }
          if (textNode) instance.texts.push({ node: textNode, parts, scope });
        }
      }
    }

    // Attribute bindings
    this.$wireAttrs(
      instance,
      meta,
      scope,
      (i) => ssrElements[i] as Element,
      true,
    );

    // Conditional bindings — use <!--wc--> markers as anchors
    if (meta.c) {
      // `meta.c` is in source order and the server renders in source order, so
      // the markers collected in document order line up one-for-one.  Indexing
      // them is what makes a block's anchor unambiguous: reconstructing it from
      // a parent plus a scan cursor is what previously let a block claim a
      // marker belonging to a nested or already-hydrated sibling.
      const condMarkers = ssrIndex.conds.length === meta.c.length ? ssrIndex.conds : null;
      for (let i = 0; i < meta.c.length; i++) {
        const [condition, blockIndex, slotMeta] = meta.c[i];
        const [parentIndex] = slotMeta;
        const blockMeta = this.$block(blockIndex);
        let condInstance: TemplateInstance | null = null;

        const marker = condMarkers ? condMarkers[i] : null;
        const ssrParent = (marker ? marker.parentNode : ssrElements[parentIndex]) ?? ssrRoot;
        let condAnchor: Comment;
        if (marker) {
          condAnchor = marker;
        } else {
          // No marker — insert anchor at the slot position
          condAnchor = document.createComment('');
          const [, beforeIndex] = slotMeta;
          const insertRef = ssrParent.childNodes[beforeIndex ?? ssrParent.childNodes.length] ?? null;
          ssrParent.insertBefore(condAnchor, insertRef);
        }
        // Trust the SSR marker range regardless of the current condition.
        // Parent bindings may not have arrived yet, so the client value can
        // temporarily disagree with SSR. An empty range must stay empty rather
        // than claiming the first static sibling after <!--/wc-->.
        if (marker && blockMeta) {
          condInstance = this.$hydrateCondContent(condAnchor, blockMeta, scope);
          if (condInstance) condInstance.parent = instance;
        }

        // Collect the <!--/wc--> end marker for deferred removal.  Do NOT remove
        // it here - later phases still need intact marker pairs to skip
        // structural block content.
        if (marker) {
          const lastNode = condInstance ? condInstance.nodes[condInstance.nodes.length - 1] : condAnchor;
          const endMarker = lastNode?.nextSibling;
          if (endMarker && endMarker.nodeType === 8 && (endMarker as Comment).data === MARKER_COND_END) {
            staleMarkers.push(endMarker);
          }
        }

        instance.conds.push({
          condition: condition as CompiledCondition, blockIndex,
          anchor: condAnchor,
          scope, owner: instance, instance: condInstance,
        });
      }
    }

    // Repeat bindings — use <!--wr--> markers as anchors, <!--wi--> for items
    if (meta.r) {
      // Indexed the same way as conditionals above.  A repeat whose collection
      // never reached the server renders no marker at all, so fall back to slot
      // positions unless every repeat in this section has one.
      const repMarkers = ssrIndex.repeats.length === meta.r.length ? ssrIndex.repeats : null;
      for (let i = 0; i < meta.r.length; i++) {
        const [collection, itemVar, blockIndex, slotMeta, keyPath] = meta.r[i];
        const [parentIndex] = slotMeta;
        const marker = repMarkers ? repMarkers[i] : null;
        const ssrParent = (marker ? marker.parentNode : ssrElements[parentIndex]) ?? ssrRoot;
        const blockMeta = this.$block(blockIndex);
        const blockTplDom = blockMeta ? getTemplateDom(blockMeta) : null;
        const rootTag = blockMeta && blockTplDom?.childNodes.length === 1 && blockTplDom.children.length === 1
          ? this.$rootTag(blockMeta)
          : null;

        let anchor: Comment;
        if (marker) {
          anchor = marker;
        } else {
          // No marker — insert anchor at the slot position for client-created content
          anchor = document.createComment('');
          const [, beforeIndex] = slotMeta;
          const tplParent = tplElements[parentIndex];
          const staticCount = tplParent ? tplParent.childNodes.length : 0;
          const insertRef = ssrParent.childNodes[Math.min(beforeIndex ?? staticCount, ssrParent.childNodes.length)] ?? null;
          ssrParent.insertBefore(anchor, insertRef);
        }
        const repeatInsts: TemplateInstance[] = [];
        const hasCollectionState = this.$hasStateRoot(collection, scope);
        const itemsArr = this.$resolveValue(collection, scope);
        const items = Array.isArray(itemsArr) ? itemsArr as unknown[] : [];

        // Collect SSR markers — single walk captures items + end boundary
        const { items: itemMarkers, end: endMarker } = marker
          ? collectItemMarkers(anchor)
          : { items: [] as Comment[], end: null as Comment | null };

        if (blockMeta && blockTplDom && anchor.parentNode && itemMarkers.length > 0) {
          if (
            !this.$activatingDeferredSSR
            && hasCollectionState
            && itemMarkers.length !== items.length
          ) {
            console.warn(
              `[webui] hydration: repeat marker count (${itemMarkers.length}) ≠ data length (${items.length}) for "${collection}"`,
            );
          }
          for (let j = 0; j < itemMarkers.length; j++) {
            const itemValue = items[j];
            const known = hasCollectionState && j < items.length;
            if (!known) this.$hasUnknownScopes = true;
            const itemScope: ScopeFrame = {
              name: itemVar,
              value: itemValue,
              parent: scope,
              known,
            };

            if (rootTag) {
              const itemEl = nextElement(itemMarkers[j]);
              if (itemEl) {
                const childInstance = this.$hydrate(itemEl, blockMeta, blockTplDom, itemScope, 1);
                childInstance.parent = instance;
                repeatInsts.push(childInstance);
              }
            } else {
              const itemParent = itemMarkers[j].parentNode;
              const nextBound = j + 1 < itemMarkers.length ? itemMarkers[j + 1] : endMarker;
              const wrapper = document.createElement('div');
              let cursor = itemMarkers[j].nextSibling;
              while (cursor && cursor !== nextBound) {
                const next = cursor.nextSibling;
                wrapper.appendChild(cursor);
                cursor = next;
              }
              const inst = this.$hydrate(wrapper, blockMeta, blockTplDom, itemScope);
              inst.parent = instance;
              inst.nodes = childNodesArray(wrapper);
              let afterNode: Node = itemMarkers[j];
              for (let nodeIndex = 0; nodeIndex < inst.nodes.length; nodeIndex++) {
                const node = inst.nodes[nodeIndex];
                itemParent?.insertBefore(node, afterNode.nextSibling);
                afterNode = node;
              }
              if (itemParent) {
                this.$replaceInstanceContainer(
                  inst,
                  wrapper,
                  itemParent as ParentNode & Node,
                );
              }
              repeatInsts.push(inst);
            }
          }

          // Defer <!--wi--> item marker removal (anchor <!--wr--> stays
          // as the runtime repeat anchor; <!--/wr--> collected below).
          for (let m = 0; m < itemMarkers.length; m++) {
            staleMarkers.push(itemMarkers[m]);
          }
        }

        // Defer <!--/wr--> end marker removal (including empty repeats).
        if (endMarker) staleMarkers.push(endMarker);

        const binding: RepeatBinding = {
          markerId: i, collection, itemVar, blockIndex,
          container: (anchor.parentNode ?? ssrRoot) as ParentNode & Node,
          start: anchor, end: null,
          scope, owner: instance, instances: repeatInsts,
          synced: hasCollectionState,
        };
        if (keyPath !== undefined) {
          binding.keyState = createRepeatKeyState(keyPath);
          if (hasCollectionState) {
            seedHydratedRepeatKeys(binding, items);
          }
        }
        instance.repeats.push(binding);
      }
    }

    // Events + refs - this is the last phase that reads the resolved index.
    this.$finalize(instance, ssrRoot, meta, (_r, i) => ssrElements[i] ?? null, scope);

    // All path-based resolution is complete. Remove the SSR markers that
    // were kept alive for structural-block skipping.  Start markers
    // (<!--wc-->, <!--wr-->) are intentionally NOT collected — they
    // remain as runtime anchors for conditional/repeat toggling.
    for (let i = 0; i < staleMarkers.length; i++) {
      staleMarkers[i].parentNode?.removeChild(staleMarkers[i]);
    }
    this.$compactNodeArray(instance);

    return instance;
  }

  // ── SSR helpers ───────────────────────────────────────────────

  /** Collect a conditional range through its matching closing marker. */
  private $collectConditionalRange(start: Comment): Node[] {
    const nodes: Node[] = [];
    let depth = 0;
    let node: Node | null = start.nextSibling;
    while (node) {
      if (node.nodeType === 8) {
        const data = (node as Comment).data;
        if (data === MARKER_COND_START) {
          depth++;
        } else if (data === MARKER_COND_END) {
          if (depth === 0) break;
          depth--;
        }
      }
      nodes.push(node);
      node = node.nextSibling;
    }
    return nodes;
  }

  /** Return whether a compiled block has structural slots beside its root element. */
  private $hasRootStructuralSlot(meta: TemplateBlockMeta): boolean {
    // Index 0 is the section root, so a slot anchored there sits outside the
    // block's own root element and rules out in-place single-root hydration.
    if (meta.c) {
      for (let i = 0; i < meta.c.length; i++) {
        if (meta.c[i][2][0] === 0) return true;
      }
    }
    if (meta.r) {
      for (let i = 0; i < meta.r.length; i++) {
        if (meta.r[i][3][0] === 0) return true;
      }
    }
    return false;
  }

  /**
   * Hydrate a conditional block's content — shared by top-level and
   * repeat-item conditional hydration paths.
   */
  private $hydrateCondContent(
    condAnchor: Comment,
    blockMeta: TemplateBlockMeta,
    scope: ScopeFrame | undefined,
  ): TemplateInstance | null {
    const rootTag = this.$rootTag(blockMeta);
    const tplDom = getTemplateDom(blockMeta);
    if (rootTag && tplDom.children.length === 1 && !this.$hasRootStructuralSlot(blockMeta)) {
      // Single-root optimisation: hydrate the element in-place (pathStart=1).
      const el = nextElement(condAnchor);
      if (el) {
        // Wire bindings only — do NOT call $updateInstance.  SSR text
        // nodes already contain correct values; evaluating bindings now
        // would overwrite them with stale data (e.g. a complex property
        // from a parent that hasn't hydrated yet).  This is consistent
        // with $mount which also skips $updateInstance for SSR roots.
        return this.$hydrate(el, blockMeta, tplDom, scope, 1);
      }
      return null;
    }
    // Multi-root, text-only, or root-level structural content needs the full range.
    const condNodes = this.$collectConditionalRange(condAnchor);
    if (condNodes.length === 0) return null;
    const wrapper = document.createElement('div');
    for (let cn = 0; cn < condNodes.length; cn++) wrapper.appendChild(condNodes[cn]);
    const inst = this.$hydrate(wrapper, blockMeta, tplDom, scope);
    inst.nodes = childNodesArray(wrapper);
    let afterNode: Node = condAnchor;
    for (let cn = 0; cn < inst.nodes.length; cn++) {
      condAnchor.parentNode?.insertBefore(inst.nodes[cn], afterNode.nextSibling);
      afterNode = inst.nodes[cn];
    }
    const container = condAnchor.parentNode as (ParentNode & Node) | null;
    if (container) this.$replaceInstanceContainer(inst, wrapper, container);
    // Same as above — trust SSR DOM, skip binding evaluation.
    return inst;
  }

  /**
   * Find existing SSR text node by mapping template text-node ordinal.
   *
   * Elements are numbered in pre-order, but text cannot be: the renderer
   * strips inter-element whitespace that `meta.h` keeps.  Text slots therefore
   * resolve by ordinal, and the SSR DOM may contain extra text nodes
   * inside structural blocks (`<if>`/`<for>`) that are not in the
   * compiled template.  We skip `<!--wc-->...<!--/wc-->` and
   * `<!--wr-->...<!--/wr-->` ranges to keep text ordinals aligned.
   */
  private $findSSRText(ssrParent: Node, tplParent: Node, beforeIndex: number): Text | null {
    // Count how many text nodes precede `beforeIndex` in the template
    const ordinals = getTplOrdinals(tplParent);
    let textOrd = 0;
    for (let k = 0; k < beforeIndex; k++) {
      const entry = ordinals.get(k);
      if (entry && entry[0] === 3) textOrd++;
    }

    // Find the matching text node in SSR DOM, skipping structural block
    // content - see findByOrdinal for the skipping algorithm.
    const found = findByOrdinal(ssrParent, 3 /* TEXT_NODE */, textOrd);
    if (found) return found as Text;

    // Fallback: any text node with content
    let child = ssrParent.firstChild;
    while (child) {
      if (child.nodeType === 3 && (child as Text).data && (child as Text).data.trim()) {
        return child as Text;
      }
      child = child.nextSibling;
    }
    return null;
  }

  /** Find the SSR insertion reference for an empty text slot. */
  private $findSSRSlotRef(ssrParent: Node, tplParent: Node, beforeIndex: number): Node | null {
    const ordinals = getTplOrdinals(tplParent);
    const children = tplParent.childNodes;
    for (let i = beforeIndex; i < children.length; i++) {
      const entry = ordinals.get(i);
      if (!entry) continue;
      return findByOrdinal(ssrParent, entry[0], entry[1]);
    }
    return null;
  }

  /** Extract root tag name from block metadata. */
  private $rootTag(meta: TemplateBlockMeta): string | null {
    let cached = rootTagCache.get(meta);
    if (cached !== undefined) return cached;
    const h = meta.h;
    if (!h || h.charCodeAt(0) !== 60) {
      rootTagCache.set(meta, null);
      return null;
    }
    let end = 1;
    while (end < h.length) {
      const c = h.charCodeAt(end);
      if (c === 32 || c === 62 || c === 47) break;
      end++;
    }
    const tag = h.slice(1, end).toLowerCase();
    rootTagCache.set(meta, tag);
    return tag;
  }

  // ═══════════════════════════════════════════════════════════════
  //  Shared: binding wiring, event wiring, refs
  // ═══════════════════════════════════════════════════════════════

  /** Wire attribute bindings using a resolver (shared by $wire and $hydrate). */
  private $wireAttrs(
    instance: TemplateInstance,
    meta: TemplateBlockMeta,
    scope: ScopeFrame | undefined,
    resolve: (index: TemplateNodeIndex) => Node | null,
    primeSSRComplexProperties = false,
  ): void {
    if (!meta.a || !meta.ag) return;
    for (let g = 0; g < meta.ag.length; g++) {
      const [targetIndex, start, count] = meta.ag[g];
      const el = resolve(targetIndex);
      if (!el || el.nodeType !== 1) continue;
      for (let j = 0; j < count; j++) {
        const entry = meta.a[start + j];
        if (!entry) continue;
        const binding = this.$makeAttr(el as Element, entry, scope);
        instance.attrs.push(binding);
        if (
          primeSSRComplexProperties &&
          binding.kind === ATTR_KIND_COMPLEX &&
          this.$attrStateIsKnown(binding)
        ) {
          this.$primeSSRComplexProperty(binding);
        }
      }
    }
  }

  /**
   * Complex properties have no HTML representation. Transfer known values
   * during SSR hydration while keeping unupgraded compiled hosts accessor-safe.
   */
  private $primeSSRComplexProperty(binding: AttrBinding): void {
    const element = binding.element;
    const value = this.$resolveValue(binding.path!, binding.scope);
    this.$writeComplexProperty(element, binding.name, value, false);
  }

  private $writeComplexProperty(
    element: Element,
    name: string,
    value: unknown,
    replayAfterHydration: boolean,
  ): void {
    const target = element as unknown as Record<string | symbol, unknown>;
    const setStateKey = target[WEBUI_SET_STATE_KEY];
    if (typeof setStateKey === 'function') {
      if (
        (setStateKey as (key: string, value: unknown) => boolean).call(
          element,
          name,
          value,
        )
      ) {
        const flush = target['$flushUpdates'];
        if (typeof flush === 'function') (flush as () => void).call(element);
        return;
      }
      target[name] = value;
      return;
    }

    if (isUnupgradedWebUITarget(element)) {
      queuePendingParentState(
        element,
        name,
        value,
        replayAfterHydration,
      );
      return;
    }

    target[name] = value;
  }

  /**
   * Hook for wiring interactivity (events + refs). This template-only base class
   * does nothing here; the interactive {@link WebUIElement} subclass overrides
   * it.
   */
  protected $finalize(
    _instance: TemplateInstance,
    _root: Node,
    _meta: TemplateBlockMeta,
    _resolver: (root: Node, index: TemplateNodeIndex) => Node | null,
    _scope?: ScopeFrame,
  ): void {}


  /** Create an AttrBinding from compiled metadata. */
  private $makeAttr(el: Element, entry: CompiledAttrMeta, scope?: ScopeFrame): AttrBinding {
    const name = entry[0];
    const kind = entry[1];
    if (kind === ATTR_KIND_BOOLEAN) return { element: el, name, kind, condition: entry[2] as CompiledCondition, scope };
    if (kind === ATTR_KIND_TEMPLATE) return { element: el, name, kind, parts: entry[2] as CompiledAttrPart[], scope };
    return { element: el, name, kind: kind as number, path: (entry[2] as string) || '', scope };
  }

  // ═══════════════════════════════════════════════════════════════
  //  Reactive update system
  // ═══════════════════════════════════════════════════════════════

  private $buildPathIndex(): void {
    if (!this.$root) return;
    const observableNames = this.$observableNames();
    const index = new Map<string, {
      texts: TextBinding[]; attrs: AttrBinding[];
      conds: CondBinding[]; repeats: RepeatBinding[];
    }>();

    const ensure = (key: string) => {
      let e = index.get(key);
      if (!e) { e = { texts: [], attrs: [], conds: [], repeats: [] }; index.set(key, e); }
      return e;
    };

    const keyFor = (path: string) => {
      const dot = path.indexOf('.');
      const root = dot > -1 ? path.slice(0, dot) : path;
      return observableNames.has(root) || this.$usesTemplateState(root) ? root : '*';
    };

    const isLocalPath = (path: string, scope?: ScopeFrame): boolean => {
      const dot = path.indexOf('.');
      const root = dot > -1 ? path.slice(0, dot) : path;
      let current = scope;
      while (current) {
        if (current.name === root) return true;
        current = current.parent;
      }
      return false;
    };

    const visit = (instance: TemplateInstance): void => {
      for (const t of instance.texts) {
        if (t.parts) {
          for (const p of t.parts) {
            if (typeof p !== 'string' && !isLocalPath(p[0], t.scope)) {
              ensure(keyFor(p[0])).texts.push(t);
            }
          }
        } else if (t.path && !isLocalPath(t.path, t.scope)) {
          ensure(keyFor(t.path)).texts.push(t);
        }
      }
      for (const a of instance.attrs) {
        if (a.path && !isLocalPath(a.path, a.scope)) {
          ensure(keyFor(a.path)).attrs.push(a);
        }
        if (a.parts) {
          for (const p of a.parts) {
            if (typeof p !== 'string' && !isLocalPath(p[0], a.scope)) {
              ensure(keyFor(p[0])).attrs.push(a);
            }
          }
        }
        if (a.condition) {
          for (const p of a.condition[1]) {
            if (!isLocalPath(p, a.scope)) ensure(keyFor(p)).attrs.push(a);
          }
        }
      }
      for (const c of instance.conds) {
        for (const p of c.condition[1]) {
          if (!isLocalPath(p, c.scope)) ensure(keyFor(p)).conds.push(c);
        }
        if (c.instance) visit(c.instance);
      }
      for (const rep of instance.repeats) {
        if (!isLocalPath(rep.collection, rep.scope)) {
          ensure(keyFor(rep.collection)).repeats.push(rep);
        }
        for (let i = 0; i < rep.instances.length; i++) {
          visit(rep.instances[i]);
        }
      }
    };
    visit(this.$root);

    // Store wildcard bindings separately — avoids duplicating them into every path
    const wc = index.get('*');
    if (wc) {
      index.delete('*');
      this.$wildcardBindings = wc;
    } else {
      this.$wildcardBindings = null;
    }
    this.$pathIndex = index;
  }

  private $updateBindings(
    texts: TextBinding[], attrs: AttrBinding[],
    conds: CondBinding[], repeats: RepeatBinding[],
    requireKnownState = false,
  ): void {
    // Fast path: with no client-absent SSR scopes (every authored component and
    // every fully-hydrated host) the walk is unnecessary, so skip it per binding.
    const gated = this.$hasUnknownScopes;
    for (let i = 0; i < texts.length; i++) {
      const binding = texts[i];
      if (
        (!gated || !binding.scope || this.$scopeIsKnown(binding.scope)) &&
        (!requireKnownState || this.$textStateIsKnown(binding))
      ) {
        this.$patchText(binding);
      }
    }
    for (let i = 0; i < attrs.length; i++) {
      const binding = attrs[i];
      if (
        (!gated || !binding.scope || this.$scopeIsKnown(binding.scope)) &&
        (!requireKnownState || this.$attrStateIsKnown(binding))
      ) {
        this.$patchAttr(binding);
      }
    }
    for (let i = 0; i < conds.length; i++) {
      const binding = conds[i];
      if (
        (!gated || !binding.scope || this.$scopeIsKnown(binding.scope)) &&
        (!requireKnownState || this.$pathsAreKnown(binding.condition[1], binding.scope))
      ) {
        this.$toggleCond(binding);
      }
    }
    for (let i = 0; i < repeats.length; i++) {
      const binding = repeats[i];
      if (
        (!gated || !binding.scope || this.$scopeIsKnown(binding.scope)) &&
        (!requireKnownState || this.$hasStateRoot(binding.collection, binding.scope))
      ) {
        syncRepeat(this, binding);
      }
    }
  }

  private $textStateIsKnown(binding: TextBinding): boolean {
    if (binding.parts) return this.$partsAreKnown(binding.parts, binding.scope);
    return !binding.path || this.$hasStateRoot(binding.path, binding.scope);
  }

  private $attrStateIsKnown(binding: AttrBinding): boolean {
    if (binding.path && !this.$hasStateRoot(binding.path, binding.scope)) return false;
    if (binding.parts && !this.$partsAreKnown(binding.parts, binding.scope)) return false;
    return !binding.condition
      || this.$pathsAreKnown(binding.condition[1], binding.scope);
  }

  private $partsAreKnown(parts: CompiledAttrPart[], scope?: ScopeFrame): boolean {
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      if (typeof part !== 'string' && !this.$hasStateRoot(part[0], scope)) return false;
    }
    return true;
  }

  private $pathsAreKnown(paths: string[], scope?: ScopeFrame): boolean {
    for (let i = 0; i < paths.length; i++) {
      if (!this.$hasStateRoot(paths[i], scope)) return false;
    }
    return true;
  }

  private $scopeIsKnown(scope: ScopeFrame): boolean {
    let frame: ScopeFrame | undefined = scope;
    while (frame) {
      if (frame.known === false) return false;
      frame = frame.parent;
    }
    return true;
  }

  $updateInstance(instance: TemplateInstance): void {
    this.$updateBindings(
      instance.texts,
      instance.attrs,
      instance.conds,
      instance.repeats,
      this.$guardUnknownState === true,
    );
  }

  private $patchText(b: TextBinding): void {
    let val: string;
    if (b.parts) {
      val = this.$resolveParts(b.parts, b.scope);
    } else if (b.path) {
      const raw = this.$resolveValue(b.path, b.scope);
      val = raw == null ? '' : String(raw);
    } else {
      return;
    }
    if (b.raw && b.rawParent) {
      // Raw binding: render unescaped HTML via innerHTML
      if (b.rawParent.innerHTML !== val) b.rawParent.innerHTML = val;
    } else {
      if (b.node.data !== val) b.node.data = val;
    }
  }

  private $patchAttr(b: AttrBinding): void {
    const el = b.element;
    switch (b.kind) {
      case ATTR_KIND_COMPLEX: {
        const v = this.$resolveValue(b.path!, b.scope);
        this.$writeComplexProperty(el, b.name, v, true);
        break;
      }
      case ATTR_KIND_BOOLEAN: {
        const show = b.condition![0](this.$resolver, b.scope);
        if (show) el.setAttribute(b.name, '');
        else el.removeAttribute(b.name);
        // Form control properties must be set via DOM property, not attribute
        if (
          (b.name === 'checked' || b.name === 'selected' || b.name === 'disabled')
          && hasNativeLiveProperty(el, b.name)
        ) {
          (el as unknown as Record<string, unknown>)[b.name] = show;
        }
        break;
      }
      case ATTR_KIND_TEMPLATE: {
        const v = this.$resolveParts(b.parts!, b.scope);
        if (el.getAttribute(b.name) !== v) el.setAttribute(b.name, v);
        break;
      }
      default: {
        const v = this.$resolveValue(b.path!, b.scope);
        const s = v == null ? '' : String(v);
        // Form control properties diverge from attributes after user interaction
        if (
          (b.name === 'checked' || b.name === 'selected')
          && hasNativeLiveProperty(el, b.name)
        ) {
          (el as unknown as Record<string, unknown>)[b.name] = !!v && v !== 'false' && v !== '0';
        } else if (b.name === 'value' && hasNativeLiveProperty(el, b.name)) {
          const target = el as Element & { value: unknown };
          if (target.value !== s) target.value = s;
        } else {
          if (el.getAttribute(b.name) !== s) el.setAttribute(b.name, s);
        }
        break;
      }
    }
  }

  private $toggleCond(c: CondBinding): void {
    const show = c.condition[0](this.$resolver, c.scope);
    if (show) {
      let created = false;
      const container = c.anchor.parentNode as (ParentNode & Node) | null;
      if (!c.instance) {
        c.instance = this.$createBlockInstance(
          c.blockIndex,
          c.scope,
          c.owner,
          container ?? undefined,
        );
        created = c.instance !== null;
      } else {
        this.$updateInstance(c.instance);
      }
      if (c.instance && container) {
        this.$insertInstanceAfter(c.anchor, container, c.instance);
        if (created) this.$changeStructure();
      }
    } else if (c.instance) {
      this.$removeInstance(c.instance);
      c.instance = null;
      this.$changeStructure(c.owner);
    }
  }

  // ── Value resolution ──────────────────────────────────────────

  $resolveValue(path: string, scope?: ScopeFrame): unknown {
    // Check scope frames first (repeat item variables)
    let frame = scope;
    while (frame) {
      if (path === frame.name) return frame.value;
      if (path.length > frame.name.length && path.charCodeAt(frame.name.length) === 46 && path.startsWith(frame.name)) {
        return dotWalk(frame.value, path, frame.name.length + 1);
      }
      frame = frame.parent;
    }
    // Resolve against component — fast path for single-segment (no dot)
    const dot = path.indexOf('.');
    if (dot === -1) return this.$resolveComponentRoot(path);
    return dotWalk(this.$resolveComponentRoot(path.substring(0, dot)), path, dot + 1);
  }

  /** Return whether a binding path's scope or component root is available. */
  $hasStateRoot(path: string, scope?: ScopeFrame): boolean {
    let frame = scope;
    while (frame) {
      if (path === frame.name
        || (path.length > frame.name.length
          && path.charCodeAt(frame.name.length) === 46
          && path.startsWith(frame.name))) {
        return frame.known !== false;
      }
      frame = frame.parent;
    }

    const dot = path.indexOf('.');
    const root = dot === -1 ? path : path.substring(0, dot);
    return hasAuthoredMember(this, root)
      || (this.$templateState !== null
        && Object.prototype.hasOwnProperty.call(this.$templateState, root));
  }

  private $resolveComponentRoot(root: string): unknown {
    const instance = this as Record<string, unknown>;
    // Template-only state wins only when the component did not author the member.
    if (
      this.$templateState &&
      Object.prototype.hasOwnProperty.call(this.$templateState, root) &&
      !hasAuthoredMember(this, root)
    ) {
      return this.$templateState[root];
    }
    return instance[root];
  }

  private $resolveParts(parts: CompiledAttrPart[], scope?: ScopeFrame): string {
    let result = '';
    for (let i = 0; i < parts.length; i++) {
      const p = parts[i];
      if (typeof p === 'string') { result += p; continue; }
      const v = this.$resolveValue(p[0], scope);
      result += v == null ? '' : String(v);
    }
    return result;
  }

  // ── Block instance management ─────────────────────────────────

  $block(blockIndex: number): TemplateBlockMeta | undefined {
    return this.$meta?.b?.[blockIndex];
  }

  $createBlockInstance(
    blockIndex: number,
    scope?: ScopeFrame,
    parent?: TemplateInstance,
    container?: ParentNode & Node,
  ): TemplateInstance | null {
    const bm = this.$block(blockIndex);
    if (!bm) return null;
    const wrapper = this.$createStagingRoot(bm);
    const inst = this.$wire(wrapper, bm, scope);
    inst.parent = parent;
    inst.nodes = childNodesArray(wrapper);
    this.$updateInstance(inst);
    if (inst.repeats.length !== 0 || inst.conds.length !== 0) {
      inst.nodes = childNodesArray(wrapper);
      this.$replaceInstanceContainer(inst, wrapper, container ?? null);
    } else if (container) {
      inst.container = container;
    }
    return inst;
  }

  $removeInstance(instance: TemplateInstance): void {
    this.$disposeInstance(instance, true);
  }

  private $disposeInstance(root: TemplateInstance, removeNodes: boolean): void {
    const stack: TemplateInstance[] = [root];
    while (stack.length > 0) {
      const instance = stack.pop();
      if (!instance) continue;
      const cleanups = instance.cleanups;
      if (cleanups) {
        for (const cleanup of cleanups) cleanup();
        cleanups.length = 0;
      }
      if (removeNodes) {
        for (const node of instance.nodes) node.parentNode?.removeChild(node);
      }
      for (const binding of instance.conds) {
        if (binding.instance) stack.push(binding.instance);
        binding.instance = null;
      }
      for (const repeat of instance.repeats) {
        for (const child of repeat.instances) stack.push(child);
        this.$clearRepeatBinding(repeat);
      }
      this.$clearInstance(instance);
    }
  }

  private $compactNodeArray(instance: TemplateInstance): void {
    const container = instance.container;
    if (!container) return;
    const nodes = instance.nodes;
    let write = 0;
    for (let read = 0; read < nodes.length; read++) {
      const node = nodes[read];
      if (node.parentNode === container) {
        nodes[write] = node;
        write++;
      }
    }
    nodes.length = write;
  }

  $changeStructure(removedFrom?: TemplateInstance): void {
    if (removedFrom) {
      let current: TemplateInstance | undefined = removedFrom;
      while (current) {
        this.$compactNodeArray(current);
        current = current.parent;
      }
    }
    this.$pathIndex = undefined;
    this.$wildcardBindings = undefined;
  }

  $insertInstanceAfter(cursor: Node | null, container: ParentNode & Node, instance: TemplateInstance): Node | null {
    this.$replaceInstanceContainer(instance, instance.container, container);
    const nodes = instance.nodes;
    if (nodes.length === 0) return cursor;
    const ref = cursor ? cursor.nextSibling : container.firstChild;
    if (nodes[0] === ref) return nodes[nodes.length - 1];
    for (let i = 0; i < nodes.length; i++) container.insertBefore(nodes[i], ref);
    return nodes[nodes.length - 1];
  }

}
