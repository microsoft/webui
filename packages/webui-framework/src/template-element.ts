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
 *   - `<!--wN-->` / `<!--/wN-->` — raw HTML ownership boundaries
 *
 * During hydration these markers are consumed: `<!--wi-->`, `<!--/wr-->`,
 * and `<!--/wc-->` are removed from the DOM. The `<!--wr-->` start
 * marker is kept as the runtime repeat anchor. A `<!--wc-->` marker remains only
 * while its conditional body is absent; visible bodies hold their position
 * directly and create an anchor lazily if they later become hidden. Raw HTML
 * markers remain as the minimum stable range required for sibling-safe reactive
 * replacement.
 *
 * One iterative traversal indexes all SSR sections before binding wiring.
 * Marker cleanup follows wiring so adjacent text and structural slots keep
 * their original boundaries throughout resolution.
 *
 * ## Comment anchors (client-created)
 *
 * For client-created components (no SSR), repeat blocks keep an empty comment
 * anchor. Conditional blocks keep one only while their body is absent.
 *
 * These comments are invisible to the user, weigh ~0 bytes, and are the
 * minimum DOM structure needed for the framework to operate.
 */

import { deferTemplateDefinition, getTemplate, templateHasFragments, templateNeedsRanges } from './template.js';
import {
  cloneTemplateContent,
  getTemplateFragment,
  getTemplateOutlets,
} from './template-content.js';
import type {
  TemplateMeta,
  TemplateBlockMeta,
  CompiledAttrMeta,
  CompiledAttrPart,
  CompiledCondition,
  CompiledRenderMeta,
  TemplateNodeIndex,
  TemplateSlot,
} from './template.js';
import { hydrationStart, hydrationEnd } from './lifecycle.js';
import {
  ACTIVATION_ACTIVATED,
  ACTIVATION_ANCESTOR_BARRIER,
  ACTIVATION_MISSING_TEMPLATE,
  ACTIVATION_STATIC_HOST_OPT_OUT,
  isStreamingHydrationMode,
  PENDING_ROOT_CONNECTED,
  STREAMED_HOST_ATTR,
  STREAMING_BOUNDARY_ACTIVATE,
} from './streaming-mode.js';
import type { ActivationOutcome } from './streaming-mode.js';
import {
  createRepeatKeyState,
  syncRepeat,
  dotWalk,
} from './element/diff.js';
import {
  collectTemplateElements,
  rawMarker,
} from './element/markers.js';
import { hydrateTemplate, type SSRIndex } from './element/hydration.js';
import { createFragmentWork, type FragmentTask, type FragmentWork } from './element/fragment-work.js';
import {
  forgetFragmentInput,
  retainFragmentInput,
  releaseFragmentInput,
  adoptFragmentInput,
  retainedFragmentInput,
  fragmentInput,
} from './fragment-inputs.js';
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
  EMPTY_BINDINGS,
  appendBinding,
  bindingArray,
  scopeSourceRoot,
  templateHasTopology,
} from './element/types.js';
import { templateHasRoot, templateRootForAttribute } from './template-roots.js';
import type {
  AttrBinding,
  CondBinding,
  RepeatBinding,
  RenderBinding,
  ScopeFrame,
  TemplateBindings,
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
// Enclose the diagnostic import in a direct positive flag check. Alias folding
// and early-return pruning happen too late to remove its module dependency.
// The `typeof` check keeps diagnostics enabled when raw ESM or tsc output runs
// without a bundler-provided flag.
declare const __WEBUI_DEV__: boolean;

// ── Caches ──────────────────────────────────────────────────────

/**
 * A dynamic node and the static DOM reference captured before wiring mutates
 * the cloned template. Raw HTML owns two nodes; every other slot owns one.
 */
interface PendingSlot {
  parent: Node;
  before: Node | null;
  order: number;
  node: Node;
  /** Paired raw-HTML end anchor. */
  end?: Comment;
}

function comparePendingSlots(left: PendingSlot, right: PendingSlot): number {
  return left.order - right.order;
}

/**
 * Insert queued dynamic nodes after all static references have been captured.
 *
 * `order` is local to slots sharing one `(parent, before)` reference. Sorting
 * the complete queue is safe because insertions at different references
 * commute; only colliding slots need a relative order.
 */
function insertPendingSlots(
  slots: PendingSlot[],
  count: number,
  needsOrdering: boolean,
): void {
  if (needsOrdering) {
    slots.length = count;
    slots.sort(comparePendingSlots);
  }
  for (let i = 0; i < count; i++) {
    const slot = slots[i];
    slot.parent.insertBefore(slot.node, slot.before);
    if (slot.end) slot.parent.insertBefore(slot.end, slot.before);
  }
}

// ── Sentinels ───────────────────────────────────────────────────

const EMPTY_SET: Set<string> = Object.freeze(new Set<string>()) as Set<string>;
/** Branded single-key state writer used by framework bindings, not public duck typing. */
const WEBUI_SET_STATE_KEY = Symbol.for('microsoft.webui.setStateKey');
/** Branded fatal-stream cleanup hook, invoked before `data-ws` is removed. */
const STREAMING_BOUNDARY_ABANDON = Symbol.for('microsoft.webui.boundaryAbandon');

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

/** Internal seam for compiler-owned hosts defined after a parent state write. */
export function consumePendingParentState(
  element: Element,
): PendingParentState | undefined {
  const pending = pendingParentStateByElement.get(element);
  if (pending) pendingParentStateByElement.delete(element);
  return pending;
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
  const attrs = meta.ta ?? EMPTY_BINDINGS;
  if (attrs.length === 0) return;

  const existing = ctor.observedAttributes ?? EMPTY_BINDINGS;
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
  declare private $fragmentWork: FragmentWork | undefined;
  declare private $fragmentKnownRoots: Set<string> | undefined;
  declare private $fragmentHydrating: boolean | undefined;
  declare private $fragmentInputVersions: Map<string, number> | undefined;
  private $templateState: Record<string, unknown> | null = null;
  private $dirtyPaths: Set<string> | null = null;
  private $pendingFlush = false;
  /** Observable paths written while connected but before hydration finished
   *  (constructor, `@observable` field initializer, or before
   *  `super.connectedCallback()`). Checked against the SSR DOM at `$ready` to
   *  surface hydration mismatches (issue #379). All access is development-only;
   *  production instances do not acquire this field. */
  declare private $preReadyWrites: Set<string> | null | undefined;
  /** State roots written while lazy SSR hydration is deferred. The values live
   *  in normal authored/template state; this set prevents older bootstrap state
   *  from replacing them and requests one synchronous replay after wiring. */
  declare private $deferredWrites: Set<string> | undefined;
  /** Keep unavailable template-only roots from being replaced by empty values
   *  after a deferred SSR activation. */
  declare private $guardUnknownState: boolean | undefined;
  /** Allocated only for components that evaluate a runtime condition. */
  declare private $resolver:
    | ((path: string, scope?: unknown) => unknown)
    | undefined;
  private $pathIndex?: Map<string, TemplateBindings>;
  /** Bindings that reference non-observable paths — updated on every flush. */
  private $wildcardBindings?: TemplateBindings | null;

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
   * a boundary containing this element has committed. Returns one of the
   * shared `ActivationOutcome` codes (`streaming-mode.ts`) so the
   * allocation-sensitive coordinator can distinguish activation, an
   * intentional static-host opt-out, missing metadata, and an unfinished
   * ancestor without per-root result objects. The union is the contract: a
   * code this method invents but the coordinator cannot decode is a compile
   * error here, and a runtime one on the coordinator side. The optional
   * `state` is this element's boundary-local SSR state, handed straight
   * through to hydration instead of via the global `window.__webui.state`
   * handoff.
   *
   * `bypassAncestor`, when supplied, is one already-resolved ancestor element
   * this root may skip exactly once while looking for its hydration barrier.
   * The coordinator owns that resolution — which compiler attributes name the
   * ancestor, and whether this root is entitled to skip it — so the
   * always-shipped bundle carries a plain identity comparison and no streaming
   * attribute names at all.
   */
  [STREAMING_BOUNDARY_ACTIVATE](
    state?: Record<string, unknown>,
    bypassAncestor?: Element,
  ): ActivationOutcome {
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
    const ancestor = this.$nearestHydrationBarrier(bypassAncestor);
    if (ancestor) {
      this.$deferredByAncestor = true;
      this.$ancestorBoundaryState = state;
      this.$hasAncestorBoundaryState = true;
      this.$registerWithHydrationBarrier(ancestor);
      return ACTIVATION_ANCESTOR_BARRIER;
    }
    // A root re-activated after its barrier lifted must not keep the stale
    // registration; the boundary state it carried is superseded by `state`.
    if (this.$deferredByAncestor) this.$clearAncestorDeferral();
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
    this.$clearAncestorDeferral();
    this.$abandonDeferredDescendants();
    this.$deferredSSR = false;
    this.$activatingDeferredSSR = false;
    this.$ready = false;
    if (typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__) {
      this.$preReadyWrites = null;
    }
    if (this.$deferredWrites) this.$deferredWrites = undefined;
  }

  /**
   * Drop any registration behind an ancestor hydration barrier.
   *
   * Shared by abandon, destroy, and re-activation so the four pieces of
   * barrier bookkeeping can never be cleared partially.
   */
  private $clearAncestorDeferral(): void {
    this.$detachDeferredAncestor();
    this.$deferredByAncestor = undefined;
    this.$ancestorBoundaryState = undefined;
    this.$hasAncestorBoundaryState = undefined;
  }

  /**
   * Hand control back to whoever retained this root, if anyone did.
   *
   * The streaming coordinator installs the hook on roots it is holding — for a
   * pending definition or a pending ancestor barrier — and owns every
   * continuation from there: eager activation, re-registration behind a
   * barrier, lazy observation, or a static-host opt-out. Re-entering ordinary
   * deferral instead can replay older page bootstrap state over a queued
   * boundary update.
   */
  private $resumeRetainedRoot(): boolean {
    const resume = (
      this as unknown as { [PENDING_ROOT_CONNECTED]?: () => void }
    )[PENDING_ROOT_CONNECTED];
    if (typeof resume !== 'function') return false;
    resume.call(this);
    return true;
  }

  /**
   * Register this constructor for a tag and install template-derived observers.
   */
  static define(tagName: string): void {
    const ctor = this as TemplateObservedConstructor;
    const meta = getTemplate(tagName);
    if (!meta) {
      // Metadata supplies template-derived observed attributes. The browser
      // snapshots that list at native define() time, so any page must wait
      // rather than permanently define an incomplete observer surface —
      // not only a streamed one. Ordinary (non-streaming) WebUI Router
      // partial navigation can register a route's template metadata after
      // an eagerly imported authored nested component module has already
      // run its top-level `define()` call; deferring here lets that later
      // registration (`registerTemplateData()`) complete the definition
      // with the full attribute list instead of leaving it stuck with only
      // the host's own compiled attributes.
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
      if (this.$resumeRetainedRoot()) return;
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
    const remountStructuralTemplate = reconnecting && !templateNeedsRanges(meta) &&
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
    if (templateNeedsRanges(meta)) this.$fragmentWork ??= createFragmentWork();
    this.$fragmentWork?.holdBudget();
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
        const fragmentWork = this.$fragmentWork;
        const fragmentState = templateHasFragments(meta);
        if (fragmentState) {
          this.$guardUnknownState = true;
          const roots = this.$fragmentKnownRoots ??= new Set();
          if (meta.tr && meta.ta) {
            for (let i = 0; i < meta.tr.length; i++) {
              if (this.hasAttribute(meta.ta[i])) roots.add(meta.tr[i]);
            }
          }
          this.$fragmentHydrating = true;
        }
        try {
          this.$root = hydrateTemplate(root, meta, this, !!fragmentWork);
        } finally {
          if (fragmentState) this.$fragmentHydrating = false;
        }
      } else {
        clientRoot = this.$createStagingRoot(meta);
        this.$root = this.$wire(clientRoot, meta, undefined, true);
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
        this.$invalidateRawValues(this.$root);
        if (this.$fragmentWork) this.$updateInstance(this.$root, true);
        else {
          this.$updateBindings(this.$root, true);
        }
      }

      // SSR only: warn when a pre-ready write left an observable disagreeing
      // with the server-rendered DOM. Client-created components have no SSR
      // content to diverge from. Production bundles remove this diagnostic.
      if (typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__) {
        if (isSSR) this.$checkHydrationMismatch();
        else this.$preReadyWrites = null;
      }

      // Client-created components: flush current attr/observable values
      // into the freshly-wired template DOM. Call $updateInstance directly
      // to avoid the $update() path-index build — it will be lazy-built
      // on the first reactive change instead.
      if (!isSSR && clientRoot) {
        this.$updateInstance(this.$root);
        const hasStructuralBindings =
          this.$root.repeats.length !== 0 || this.$root.conds.length !== 0 ||
          this.$root.renders !== undefined;
        if (hasStructuralBindings && !this.$root.range && this.$root.nodes !== EMPTY_BINDINGS) {
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
                if (hasStructuralBindings && !mountedInstance.range && mountedInstance.nodes !== EMPTY_BINDINGS) {
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
      this.$fragmentWork?.releaseBudget();
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
    const installation = installComponentStyles(this.localName, styleTarget, this);
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
    if (this.$fragmentWork) this.$fragmentWork.stack.length = 0;
    const shadowRoot = this.shadowRoot;
    if (shadowRoot && cancelTemplateLinkStyleMount(shadowRoot)) {
      this.$resetClientShadow = true;
    }
    if (!this.$root) {
      this.$clearAncestorDeferral();
      this.$abandonDeferredDescendants();
      this.$deferredSSR = false;
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
    if (typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__) {
      this.$preReadyWrites = null;
    }
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
    instance.alive = false;
    instance.scope = undefined;
    instance.parent = undefined;
    instance.container = null;
    if (instance.nodes.length > 0) instance.nodes.length = 0;
    if (instance.texts.length > 0) instance.texts.length = 0;
    if (instance.attrs.length > 0) instance.attrs.length = 0;
    if (instance.conds.length > 0) instance.conds.length = 0;
    if (instance.repeats.length > 0) instance.repeats.length = 0;
    if (instance.renders?.length) instance.renders.length = 0;
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

  /**
   * Walk up to the nearest ancestor that must hydrate before this instance.
   *
   * `bypassAncestor` is an opt-in, coordinator-resolved escape hatch: exactly
   * one occurrence of that element is stepped over, which is how an early
   * streamed child hydrates ahead of the still-unfinished component host that
   * encloses it. Every other barrier — including a second, outer one — is
   * still honoured, so parent-first ordering holds.
   */
  private $nearestHydrationBarrier(
    bypassAncestor?: Element,
  ): Element | undefined {
    let bypass = bypassAncestor;
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
      if (parent === bypass) {
        bypass = undefined;
        current = parent;
        continue;
      }
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
    if (this.$resumeRetainedRoot()) return;
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
    if (typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__) {
      this.$preReadyWrites = null;
    }
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
    if (this.$meta && templateHasFragments(this.$meta)) {
      (this.$fragmentKnownRoots ??= new Set()).add(key);
    }
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
    const fragments = this.$meta && templateHasFragments(this.$meta);
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (deferredWrites?.has(key)) continue;
      if (observableNames.has(key)) {
        if (!this.$shouldApplySSRState(key)) continue;
        // Write to backing field directly — no reactive update yet
        (this as Record<string, unknown>)[`_${key}`] = state[key];
        if (fragments) (this.$fragmentKnownRoots ??= new Set()).add(key);
      } else if (this.$usesTemplateState(key) && this.$shouldApplyTemplateStateFromSSR(key)) {
        this.$writeTemplateState(key, state[key]);
        if (fragments) (this.$fragmentKnownRoots ??= new Set()).add(key);
      }
    }
  }

  /**
   * Apply instance-local state supplied by an SSR parent before this child
   * walks its own bindings. Parent values override page-wide bootstrap keys.
   */
  private $applyPendingParentState(replayAfterHydration: boolean): void {
    const pending = consumePendingParentState(this);
    if (!pending) return;

    const state = pending.values;
    const observableNames = this.$observableNames();
    const keys = Object.keys(state);
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (this.$meta && templateHasFragments(this.$meta)) {
        (this.$fragmentKnownRoots ??= new Set()).add(key);
      }
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
    // A pathless update is a lifecycle refresh — a reconnect, or a caller
    // asking for a full re-render — not a write to any input. Only a real
    // write invalidates a captured alias, so a synchronous reparent (remove
    // and re-append in the same task, which never reaches delayed teardown)
    // keeps resolving the input its invocation was streamed with.
    const fragments = this.$fragmentWork && this.$meta && templateHasFragments(this.$meta);
    if (path && this.$fragmentInputVersions) this.$advanceFragmentInput(path);
    if (fragments && path && (this.$ready || this.$hasMounted || this.$deferredSSR)) {
      const dot = path.indexOf('.');
      (this.$fragmentKnownRoots ??= new Set()).add(dot === -1 ? path : path.slice(0, dot));
    }
    if (!this.$ready || !this.$root) {
      if (this.$deferredClientMount) return;
      if (path && this.$deferredSSR) this.$recordDeferredWrite(path);
      // A reactive write arrived while connected but before hydration
      // completed. `$update` cannot touch the DOM yet, so record the path and
      // check it against the SSR DOM once hydrated (see #379). The flag gates the
      // recording so production bundles never allocate the tracking Set.
      if ((typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__) && path && this.isConnected) {
        (this.$preReadyWrites ??= new Set()).add(path);
      }
      return;
    }

    // Lazy-build path index on first update (deferred from hydration)
    if (!this.$pathIndex) this.$buildPathIndex();

    if (path) {
      if (this.$pathIndex?.has(path) || (fragments && this.$wildcardBindings)) {
        // Batch path-specific updates via microtask coalescing.
        (this.$dirtyPaths ??= new Set()).add(path);
        if (!this.$pendingFlush) {
          this.$pendingFlush = true;
          queueMicrotask(() => this.$flush());
        }
        return;
      }
      if (fragments) return;
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
    for (const path of writes) this.$advanceFragmentInput(path);
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
    const work = this.$fragmentWork;
    if (work?.active) return;
    if (!this.$ready || !this.$root || !this.$dirtyPaths?.size) {
      this.$dirtyPaths = null;
      this.$pendingFlush = false;
      return;
    }
    if (!this.$pathIndex) this.$buildPathIndex();
    if (!this.$pathIndex) return;

    work?.begin();
    try {
      while (this.$dirtyPaths && this.$dirtyPaths.size > 0) {
        // Reentrant setters start a new pass in this same operation.
        const dirty = this.$dirtyPaths;
        this.$dirtyPaths = null;
        for (const path of dirty) {
          if (!this.$pathIndex) this.$buildPathIndex();
          const entry = this.$pathIndex?.get(path);
          if (entry) this.$updateBindings(entry, requireKnownState);
        }
        // Wildcard bindings participate once per pass, not once per dirty path.
        if (!this.$pathIndex) this.$buildPathIndex();
        if (this.$wildcardBindings) {
          this.$updateBindings(this.$wildcardBindings, requireKnownState);
        }
        if (work) {
          work.sort();
          work.drain(this, requireKnownState);
          if (this.$dirtyPaths) work.nextPass();
        }
        if (!work && !this.$pathIndex) this.$buildPathIndex();
      }
    } finally {
      work?.end();
      this.$pendingFlush = false;
    }
  }

  /** Execute one graph work item; descendants only enqueue further work. */
  $processFragmentTask(
    task: FragmentTask,
    requireKnownState = false,
  ): void {
    const work = this.$fragmentWork!;
    requireKnownState ||= this.$guardUnknownState === true;
    // Range-only templates retain ordinary whole-scope protection.
    if (this.$hasUnknownScopes && task.scope && !this.$scopeIsKnown(task.scope)
      && this.$meta && !templateHasFragments(this.$meta)) return;
    if ('texts' in task) {
      for (let i = task.attrs.length - 1; i >= 0; i--) work.enqueue(task.attrs[i]);
      for (let i = task.texts.length - 1; i >= 0; i--) work.enqueue(task.texts[i]);
      for (let i = task.repeats.length - 1; i >= 0; i--) work.enqueue(task.repeats[i]);
      for (let i = task.conds.length - 1; i >= 0; i--) work.enqueue(task.conds[i]);
      if (task.renders) {
        for (let i = task.renders.length - 1; i >= 0; i--) work.enqueue(task.renders[i]);
      }
    } else if ('node' in task) {
      if (!requireKnownState || this.$textStateIsKnown(task)) this.$patchText(task);
    } else if ('element' in task) {
      if (!requireKnownState || this.$attrStateIsKnown(task)) this.$patchAttr(task);
    } else if ('collection' in task) {
      if (!requireKnownState || this.$hasStateRoot(task.collection, task.scope)) syncRepeat(this, task);
    } else if ('condition' in task) {
      if (!requireKnownState || this.$pathsAreKnown(task.condition[1], task.scope)) this.$toggleCond(task);
    } else {
      this.$syncRender(task);
    }
  }

  private $makeRender(
    owner: TemplateInstance,
    meta: CompiledRenderMeta,
    anchor: Comment,
    end: Comment,
    hydrate: boolean,
    sourceId?: number,
  ): RenderBinding {
    const binding: RenderBinding = {
      blockIndex: meta[0], anchor, end, owner, instance: null, scope: owner.scope,
    };
    if (meta.length === 4) {
      binding.path = meta[2];
      binding.alias = {
        name: meta[3], value: undefined, known: false, isAlias: true,
        sourceRoot: scopeSourceRoot(meta[2], owner.scope),
      };
      if (hydrate && !this.$adoptFragmentInput(binding, sourceId)) {
        this.$refreshRenderScope(binding, true);
      }
    }
    return binding;
  }

  private $refreshRenderScope(binding: RenderBinding, allowUnknown: boolean): void {
    if (!binding.path || !binding.alias) return;
    if (!this.$hasStateRoot(binding.path, binding.scope)) {
      if (allowUnknown) {
        binding.alias.known = false;
        return;
      }
      throw new Error(`[WebUI] Missing fragment scope "${binding.path}"; provide the caller state path.`);
    }
    const value = this.$resolveValue(binding.path, binding.scope);
    if (value === undefined) {
      throw new Error(`[WebUI] Missing fragment scope "${binding.path}"; provide every path segment.`);
    }
    binding.alias.value = value;
    binding.alias.known = true;
    binding.captureVersion = undefined;
    forgetFragmentInput(binding.anchor);
    this.$releaseFragmentInput(binding);
  }

  private $advanceFragmentInput(path: string): void {
    const versions = this.$fragmentInputVersions;
    if (!versions) return;
    const dot = path.indexOf('.');
    const root = dot < 0 ? path : path.slice(0, dot);
    if (versions.has(root)) versions.set(root, versions.get(root)! + 1);
  }

  private $adoptFragmentInput(binding: RenderBinding, sourceId?: number): boolean {
    const alias = binding.alias!;
    const remembered = retainedFragmentInput(binding.anchor);
    let value: unknown;
    if (sourceId !== undefined) {
      if (binding.sourceId === sourceId) {
        // Re-resolving an identifier this binding already claimed must not
        // count a second adopter, or the claim could never be released.
        value = fragmentInput(this, sourceId, binding.anchor);
      } else {
        this.$releaseFragmentInput(binding);
        if (!remembered) {
          value = adoptFragmentInput(this, sourceId, binding.anchor);
          binding.sourceId = sourceId;
        }
      }
    }
    if (value === undefined && !remembered) return false;
    this.$fragmentInputVersions ??= new Map();
    const version = this.$fragmentInputVersion(alias.sourceRoot!, true);
    if (remembered && remembered.version !== version) return false;
    alias.value = remembered ? remembered.scope.value : value;
    alias.known = true;
    binding.captureVersion = version;
    return true;
  }

  private $fragmentInputVersion(roots: string | readonly string[], register = false): number {
    const versions = this.$fragmentInputVersions!;
    if (typeof roots === 'string') {
      const version = versions.get(roots) ?? 0;
      if (register) versions.set(roots, version);
      return version;
    }
    let sum = 0;
    for (const root of roots) {
      const version = versions.get(root) ?? 0;
      if (register) versions.set(root, version);
      sum += version;
    }
    return sum;
  }

  /** Abandon this invocation's claim on a reserved input, if it holds one. */
  private $releaseFragmentInput(binding: RenderBinding): void {
    const id = binding.sourceId;
    if (id === undefined) return;
    binding.sourceId = undefined;
    releaseFragmentInput(this, id);
  }

  private $syncRender(binding: RenderBinding): void {
    const depth = (binding.owner.callDepth ?? 0) + 1;
    this.$fragmentWork!.visit(depth);
    if (
      binding.captureVersion === undefined
      || binding.captureVersion !== this.$fragmentInputVersion(binding.alias!.sourceRoot!)
    ) {
      this.$refreshRenderScope(binding, this.$guardUnknownState === true && binding.instance !== null);
    }
    if (binding.instance) {
      this.$updateInstance(binding.instance);
      return;
    }
    const container = binding.anchor.parentNode as (ParentNode & Node) | null;
    if (!container) return;
    const instance = this.$createBlockInstance(binding.blockIndex, binding.alias, binding.owner, container);
    if (!instance) throw new Error(`[WebUI] Missing compiled fragment block ${binding.blockIndex}; rebuild the template.`);
    instance.callDepth = depth;
    binding.instance = instance;
    this.$insertInstanceAfter(binding.anchor, container, instance);
    this.$changeStructure();
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
  // Guard the method itself so the sole diagnostic import becomes unreachable
  // during production bundling even though the class method remains present.

  private $checkHydrationMismatch(): void {
    if (typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__) {
      const writes = this.$preReadyWrites;
      this.$preReadyWrites = null;
      if (!writes || writes.size === 0 || !this.$root) return;
      if (!this.$pathIndex) this.$buildPathIndex();
      const index = this.$pathIndex;
      if (!index) return;
      const ctx: MismatchContext = {
        resolver: this.$conditionResolver(),
        resolveParts: (parts, scope) => this.$resolveParts(parts, scope),
        resolveValue: (path, scope) => this.$resolveValue(path, scope),
      };
      const tag = this.tagName.toLowerCase();
      // Capture the read-only comparison inputs before the import's microtask.
      void import('./hydration-mismatch.js').then((m) =>
        m.reportHydrationMismatch(tag, writes, index, ctx),
      );
    }
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
      if (current.renders) {
        for (let i = 0; i < current.renders.length; i++) {
          const child = current.renders[i].instance;
          if (child) stack.push(child);
        }
      }
    }
  }

  // ═══════════════════════════════════════════════════════════════
  //  Client-created wiring — exact childNode index resolution
  // ═══════════════════════════════════════════════════════════════

  private $wire(root: Node, meta: TemplateBlockMeta, scope?: ScopeFrame, componentRoot = false): TemplateInstance {
    const instance: TemplateInstance = {
      scope, container: root as ParentNode & Node,
      nodes: componentRoot && !this.$fragmentWork && !templateHasTopology(meta)
        ? EMPTY_BINDINGS : childNodesArray(root),
      texts: bindingArray<TextBinding>(meta.tx?.length ?? 0),
      attrs: bindingArray<AttrBinding>(meta.a?.length ?? 0),
      conds: bindingArray<CondBinding>(meta.c?.length ?? 0),
      repeats: bindingArray<RepeatBinding>(meta.r?.length ?? 0),
    };
    if (this.$fragmentWork) {
      instance.range = true;
      instance.alive = true;
    }

    // Resolve every static insertion reference before inserting dynamic nodes.
    // Any insertion shifts childNodes and invalidates later compiled offsets.
    //
    // Cloned template DOM matches `h` exactly, so numbering its elements in
    // pre-order reproduces the indices the compiler assigned.
    const elements = collectTemplateElements(root);

    const pendingSlots = new Array<PendingSlot>(
      (meta.tx?.length ?? 0) + (meta.c?.length ?? 0) + (meta.r?.length ?? 0) + (meta.u?.length ?? 0),
    );
    let pendingSlotCount = 0;
    let needsSlotOrdering = false;

    // Text bindings: create direct nodes and queue their compiled placement.
    let rawIndex = 0;
    if (meta.tx) {
      for (let i = 0; i < meta.tx.length; i++) {
        const entry = meta.tx[i];
        const [slot, parts] = entry;
        const raw = entry[2] === 1;
        const [parentIndex, beforeIndex, order = 0] = slot;
        const parent = elements[parentIndex];
        if (!parent || (parent.nodeType !== 1 && parent.nodeType !== 11)) continue;
        let node: Node;
        let end: Comment | undefined;
        if (raw) {
          const start = document.createComment(rawMarker(rawIndex));
          end = document.createComment(rawMarker(rawIndex, true));
          rawIndex++;
          instance.texts.push({
            node: start,
            parts,
            scope,
            raw: true,
            rawEnd: end,
            rawOwner: instance,
          });
          node = start;
        } else {
          const textNode = document.createTextNode('');
          instance.texts.push({ node: textNode, parts, scope });
          node = textNode;
        }
        pendingSlots[pendingSlotCount++] = {
          parent,
          before: parent.childNodes[beforeIndex] || null,
          order,
          node,
          end,
        };
        if (order > 0) needsSlotOrdering = true;
      }
    }

    // Conditional bindings: create insertion anchors and queue their placement.
    // Visible blocks release these anchors during the first binding pass.
    if (meta.c) {
      for (let i = 0; i < meta.c.length; i++) {
        const [condition, blockIndex, slotMeta] = meta.c[i];
        const [parentIndex, beforeIndex, order = 0] = slotMeta;
        const parent = elements[parentIndex];
        if (!parent || (parent.nodeType !== 1 && parent.nodeType !== 11)) continue;
        const anchor = document.createComment('');
        const end = this.$fragmentWork ? document.createComment('') : undefined;
        instance.conds.push({
          condition: condition as CompiledCondition,
          blockIndex,
          anchor,
          scope,
          owner: instance,
          instance: null,
          ...(end ? { end } : {}),
        });
        pendingSlots[pendingSlotCount++] = {
          parent,
          before: parent.childNodes[beforeIndex] || null,
          order,
          node: anchor,
          end,
        };
        if (order > 0) needsSlotOrdering = true;
      }
    }

    // Repeat bindings: create stable anchors and queue their placement.
    if (meta.r) {
      for (let i = 0; i < meta.r.length; i++) {
        const [collection, itemVar, blockIndex, slotMeta, keyPath] = meta.r[i];
        const [parentIndex, beforeIndex, order = 0] = slotMeta;
        const parent = elements[parentIndex];
        if (!parent || (parent.nodeType !== 1 && parent.nodeType !== 11)) continue;
        const anchor = document.createComment('');
        const binding: RepeatBinding = {
          markerId: i, collection, itemVar, blockIndex,
          container: parent as ParentNode & Node, start: anchor,
          end: this.$fragmentWork ? document.createComment('') : null,
          scope, owner: instance, instances: [],
        };
        if (keyPath !== undefined) {
          binding.keyState = createRepeatKeyState(keyPath);
        }
        instance.repeats.push(binding);
        pendingSlots[pendingSlotCount++] = {
          parent,
          before: parent.childNodes[beforeIndex] || null,
          order,
          node: anchor,
          end: binding.end ?? undefined,
        };
        if (order > 0) needsSlotOrdering = true;
      }

    }
    if (meta.u) {
      instance.renders = [];
      for (let i = 0; i < meta.u.length; i++) {
        const entry = meta.u[i];
        const [parentIndex, beforeIndex, order = 0] = entry[1];
        const parent = elements[parentIndex];
        if (!parent) continue;
        const anchor = document.createComment('');
        const end = document.createComment('');
        instance.renders.push(this.$makeRender(instance, entry, anchor, end, false));
        pendingSlots[pendingSlotCount++] = {
          parent, before: parent.childNodes[beforeIndex] ?? null,
          order, node: anchor, end,
        };
        if (order > 0) needsSlotOrdering = true;
      }
    }

    // Co-located slots share one static insertion reference. Commit them only
    // after every reference has been captured from the untouched DOM.
    insertPendingSlots(pendingSlots, pendingSlotCount, needsSlotOrdering);

    const outlets = getTemplateOutlets(meta);
    if (outlets) {
      for (let i = 0; i < outlets.length; i++) {
        const index = outlets[i];
        const element = elements[index];
        if (!element || element.nodeType !== 1 || !element.parentNode) {
          throw new Error('[WebUI] Invalid template outlet. Rebuild the server and client templates together.');
        }
        const start = document.createComment('wo');
        const end = document.createComment('/wo');
        (element as Element).replaceWith(start, end);
        elements[index] = start;
        if (!this.$fragmentWork && instance.nodes !== EMPTY_BINDINGS) {
          const position = instance.nodes.indexOf(element);
          if (position >= 0) instance.nodes.splice(position, 1, start, end);
        }
      }
    }

    // Pre-collected element references remain stable after slot insertion.
    this.$wireAttrs(instance, meta, scope, (i) => elements[i] ?? null);
    this.$finalize(
      instance, root, meta, (_r, i) => elements[i] ?? null, scope,
      this.$fragmentWork ? elements : undefined,
    );

    // Create conditional blocks immediately; the first full binding pass
    // reconciles repeats after this wiring step returns.
    if (this.$fragmentWork) {
      for (let i = 0; i < instance.texts.length; i++) instance.texts[i].owner = instance;
      for (let i = 0; i < instance.attrs.length; i++) instance.attrs[i].owner = instance;
      instance.nodes = childNodesArray(root);
    } else {
      for (let i = 0; i < instance.conds.length; i++) this.$toggleCond(instance.conds[i]);
    }

    return instance;
  }

  // ═══════════════════════════════════════════════════════════════
  //  SSR hydration — marker-based in-place DOM matching
  // ═══════════════════════════════════════════════════════════════

  /** Build once on a shape-cache miss; the hydrator owns the only element table. */
  $templateElements(meta: TemplateBlockMeta): Array<Node | undefined> {
    return collectTemplateElements(getTemplateFragment(meta));
  }

  /** Charge actual calls, never DOM depth or repeat item count. */
  $visitHydrationInvocation(depth: number): void {
    this.$fragmentWork!.visit(depth);
  }

  /** Create an SSR call scope without evaluating its body. */
  $createHydrationRender(
    owner: TemplateInstance,
    meta: CompiledRenderMeta,
    anchor: Comment,
    end: Comment,
    sourceId?: number,
  ): RenderBinding {
    return this.$makeRender(owner, meta, anchor, end, true, sourceId);
  }

  /** Preserve unknown item scopes and the ordinary hydration diagnostic. */
  $hydratedRepeat(binding: RepeatBinding, items: unknown[], known: boolean): void {
    if (binding.instances.length > (known ? items.length : 0)) this.$hasUnknownScopes = true;
    if (!(this.$meta && templateHasFragments(this.$meta)) && !this.$activatingDeferredSSR && known &&
      binding.instances.length !== items.length && binding.instances.length > 0) {
      console.warn(
        `[webui] hydration: repeat marker count (${binding.instances.length}) ≠ data length (${items.length}) for "${binding.collection}"`,
      );
    }
  }

  /** Wire leaf bindings and host integration once, after all SSR ranges validate. */
  $wireHydrationSection(
    instance: TemplateInstance,
    meta: TemplateBlockMeta,
    index: SSRIndex,
  ): void {
    const scope = instance.scope;
    let rawIndex = 0;
    if (meta.tx?.length) {
      const texts = new Array<TextBinding>(meta.tx.length);
      let textCount = 0;
      for (let i = 0; i < meta.tx.length; i++) {
        const [slot, parts, successor] = meta.tx[i];
        const parent = index.elements[slot[0]];
        if (!parent) continue;
        if (successor === 1) {
          const range = index.raws[rawIndex++];
          const binding: TextBinding = {
            node: range[0], rawEnd: range[1], raw: true, parts, scope,
            rawOwner: instance,
          };
          if (instance.range) binding.owner = instance;
          if ((!scope || this.$scopeIsKnown(scope)) && this.$textStateIsKnown(binding)) {
            binding.rawValue = this.$resolveParts(parts, scope);
          }
          texts[textCount++] = binding;
          continue;
        }
        const ref = this.$ssrSlotRef(index, slot, successor ?? 0);
        const previous = ref ? ref.previousSibling : parent.lastChild;
        let node = previous?.nodeType === 3 && previous !== index.start ? previous as Text : null;
        if (!node) {
          node = document.createTextNode('');
          parent.insertBefore(node, ref);
          let owner: TemplateInstance | undefined = instance;
          while (owner && parent === owner.container && owner.nodes !== EMPTY_BINDINGS) {
            const offset = ref ? owner.nodes.indexOf(ref) : -1;
            if (offset === -1) owner.nodes.push(node);
            else owner.nodes.splice(offset, 0, node);
            if (instance.range) break;
            owner = owner.parent;
          }
        }
        const binding: TextBinding = { node, parts, scope };
        if (instance.range) binding.owner = instance;
        texts[textCount++] = binding;
      }
      if (textCount !== texts.length) texts.length = textCount;
      // Publish only the completed, dense group before property/event wiring.
      instance.texts = textCount > 0 ? texts : EMPTY_BINDINGS;
    }
    this.$wireAttrs(instance, meta, scope, i => index.elements[i] ?? null, true);
    if (instance.range) {
      for (let i = 0; i < instance.attrs.length; i++) instance.attrs[i].owner = instance;
    }
    this.$finalize(
      instance, instance.container!, meta, (_root, i) => index.elements[i] ?? null,
      scope, index.elements,
    );
  }

  private $ssrSlotRef(
    index: SSRIndex,
    slot: TemplateSlot,
    successor: number,
  ): Node | null {
    if (successor === 0) return slot[0] === 0 ? index.end : null;
    const offset = Math.floor(successor / 8);
    switch (successor % 8) {
      case 2: return index.conds[offset];
      case 3: return index.repeats[offset];
      case 4: return index.renders[offset];
      case 5: return index.raws[offset][0];
      case 6: return index.elements[offset + 1]!;
      case 7: return index.comments[slot[0]]![offset];
      default: throw new Error('[WebUI] Invalid compiled text successor. Rebuild server and client templates together.');
    }
  }

  /** Return whether a compiled block has structural slots beside its root element. */
  private $hasRootStructuralSlot(meta: TemplateBlockMeta): boolean {
    // Root-level dynamic ranges may leave detached entries in an ordinary
    // ancestor's flattened node list when a conditional is removed.
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
    if (meta.tx) {
      for (let i = 0; i < meta.tx.length; i++) {
        if (meta.tx[i][2] === 1 && meta.tx[i][0][0] === 0) return true;
      }
    }
    return false;
  }

  // ═══════════════════════════════════════════════════════════════
  //  Shared: binding wiring, event wiring, refs
  // ═══════════════════════════════════════════════════════════════

  /** Wire attribute bindings using a resolver for client and SSR sections. */
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
    _elements?: Array<Node | undefined>,
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
    const index = new Map<string, TemplateBindings>();

    const ensure = (key: string) => {
      let e = index.get(key);
      if (!e) {
        e = {
          texts: EMPTY_BINDINGS, attrs: EMPTY_BINDINGS,
          conds: EMPTY_BINDINGS, repeats: EMPTY_BINDINGS,
        };
        index.set(key, e);
      }
      return e;
    };
    const keyFor = (path: string) => {
      const dot = path.indexOf('.');
      const root = dot > -1 ? path.slice(0, dot) : path;
      return observableNames.has(root) || this.$usesTemplateState(root) ? root : '*';
    };

    const isLocalPath = (path: string, scope?: ScopeFrame, input = false): boolean => {
      const dot = path.indexOf('.');
      const root = dot > -1 ? path.slice(0, dot) : path;
      let current = scope;
      while (current) {
        if (current.name === root) {
          return current.isAlias === true || dot === -1
            || (!input && !observableNames.has(root) && !this.$usesTemplateState(root));
        }
        current = current.parent;
      }
      return false;
    };

    const stack: TemplateInstance[] = [this.$root];
    let order = 0;
    while (stack.length > 0) {
      const instance = stack.pop()!;
      if (this.$fragmentWork) instance.order = order++;
      for (const t of instance.texts) {
        if (t.parts) {
          for (const p of t.parts) {
            if (typeof p !== 'string' && !isLocalPath(p[0], t.scope)) {
              const entry = ensure(keyFor(p[0]));
              entry.texts = appendBinding(entry.texts, t);
            }
          }
        } else if (t.path && !isLocalPath(t.path, t.scope)) {
          const entry = ensure(keyFor(t.path));
          entry.texts = appendBinding(entry.texts, t);
        }
      }
      for (const a of instance.attrs) {
        if (a.path && !isLocalPath(a.path, a.scope)) {
          const entry = ensure(keyFor(a.path));
          entry.attrs = appendBinding(entry.attrs, a);
        }
        if (a.parts) {
          for (const p of a.parts) {
            if (typeof p !== 'string' && !isLocalPath(p[0], a.scope)) {
              const entry = ensure(keyFor(p[0]));
              entry.attrs = appendBinding(entry.attrs, a);
            }
          }
        }
        if (a.condition) {
          for (const p of a.condition[1]) {
            if (!isLocalPath(p, a.scope)) {
              const entry = ensure(keyFor(p));
              entry.attrs = appendBinding(entry.attrs, a);
            }
          }
        }
      }
      for (const c of instance.conds) {
        for (const p of c.condition[1]) {
          if (!isLocalPath(p, c.scope)) {
            const entry = ensure(keyFor(p));
            entry.conds = appendBinding(entry.conds, c);
          }
        }
      }
      for (const rep of instance.repeats) {
        if (!isLocalPath(rep.collection, rep.scope)) {
          const entry = ensure(keyFor(rep.collection));
          entry.repeats = appendBinding(entry.repeats, rep);
        }
      }
      if (instance.renders) {
        for (let i = instance.renders.length - 1; i >= 0; i--) {
          const render = instance.renders[i];
          if (render.path && !isLocalPath(render.path, render.scope, true)) {
            const dot = render.path.indexOf('.');
            const root = dot === -1 ? render.path : render.path.slice(0, dot);
            const entry = ensure(root);
            entry.renders = appendBinding(entry.renders ?? EMPTY_BINDINGS, render);
          }
          if (render.instance) stack.push(render.instance);
        }
      }
      for (let i = instance.repeats.length - 1; i >= 0; i--) {
        const children = instance.repeats[i].instances;
        for (let j = children.length - 1; j >= 0; j--) stack.push(children[j]);
      }
      for (let i = instance.conds.length - 1; i >= 0; i--) {
        const child = instance.conds[i].instance;
        if (child) stack.push(child);
      }
    }

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
    bindings: TemplateBindings,
    requireKnownState = false,
  ): void {
    const { texts, attrs, conds, repeats, renders } = bindings;
    if (this.$fragmentWork) {
      const work = this.$fragmentWork;
      const outer = work.begin();
      try {
        // The LIFO operation stack executes each owner's groups in binding order.
        if (renders) for (let i = renders.length - 1; i >= 0; i--) work.enqueue(renders[i]);
        for (let i = repeats.length - 1; i >= 0; i--) work.enqueue(repeats[i]);
        for (let i = conds.length - 1; i >= 0; i--) work.enqueue(conds[i]);
        for (let i = attrs.length - 1; i >= 0; i--) work.enqueue(attrs[i]);
        for (let i = texts.length - 1; i >= 0; i--) work.enqueue(texts[i]);
        if (outer) {
          work.sort();
          work.drain(this, requireKnownState);
        }
      } finally {
        if (outer) work.end();
      }
      return;
    }
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

  private $invalidateRawValues(root: TemplateInstance): void {
    const stack: TemplateInstance[] = [root];
    while (stack.length > 0) {
      const instance = stack.pop();
      if (!instance) continue;
      for (let i = 0; i < instance.texts.length; i++) {
        const binding = instance.texts[i];
        if (binding.raw) binding.rawValue = undefined;
      }
      for (let i = 0; i < instance.conds.length; i++) {
        const child = instance.conds[i].instance;
        if (child) stack.push(child);
      }
      for (let i = 0; i < instance.repeats.length; i++) {
        const children = instance.repeats[i].instances;
        for (let j = 0; j < children.length; j++) stack.push(children[j]);
      }
      if (instance.renders) {
        for (let i = 0; i < instance.renders.length; i++) {
          const child = instance.renders[i].instance;
          if (child) stack.push(child);
        }
      }
    }
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

  $updateInstance(
    instance: TemplateInstance,
    requireKnownState = this.$guardUnknownState === true,
  ): void {
    if (this.$fragmentWork) {
      const work = this.$fragmentWork;
      const outer = work.begin();
      try {
        work.enqueue(instance);
        if (outer) work.drain(this, requireKnownState);
      } finally {
        if (outer) work.end();
      }
      return;
    }
    this.$updateBindings(instance, requireKnownState);
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
    if (b.raw && b.rawEnd) {
      if (b.rawValue === val) return;
      const range = document.createRange();
      range.setStartAfter(b.node);
      range.setEndBefore(b.rawEnd);
      range.deleteContents();
      let rawNodes: Node[] | undefined;
      const rawOwner = b.rawOwner;
      if (val !== '') {
        const fragment = range.createContextualFragment(val);
        if (rawOwner && !rawOwner.range && b.node.parentNode === rawOwner.container) {
          rawNodes = childNodesArray(fragment);
        }
        range.insertNode(fragment);
      } else if (rawOwner && !rawOwner.range && b.node.parentNode === rawOwner.container) {
        rawNodes = [];
      }
      if (rawNodes && rawOwner) {
        const container = b.node.parentNode;
        let owner: TemplateInstance | undefined = rawOwner;
        while (owner && owner.container === container) {
          const ownerNodes = owner.nodes;
          const startIndex = ownerNodes.indexOf(b.node);
          const endIndex = ownerNodes.indexOf(b.rawEnd, startIndex + 1);
          if (startIndex >= 0 && endIndex > startIndex) {
            ownerNodes.splice(startIndex + 1, endIndex - startIndex - 1, ...rawNodes);
          }
          owner = owner.parent;
        }
      }
      b.rawValue = val;
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
        const show = b.condition![0](this.$conditionResolver(), b.scope);
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

  /** Swap one live range in every owner that shares its DOM container. */
  private $swapOwnedRange(
    owner: TemplateInstance,
    current: Node | readonly Node[],
    replacement: Node | readonly Node[],
  ): void {
    const currentIsRange = Array.isArray(current);
    const first = currentIsRange ? current[0] : current as Node;
    if (!first) return;
    const count = currentIsRange ? current.length : 1;
    const last = currentIsRange ? current[count - 1] : first;
    const container = first.parentNode;
    let instance: TemplateInstance | undefined = owner;
    while (container && instance && instance.container === container) {
      const nodes = instance.nodes;
      const index = nodes.indexOf(first);
      if (index >= 0 && nodes[index + count - 1] === last) {
        if (Array.isArray(replacement)) {
          nodes.splice(index, count, ...replacement);
        } else {
          nodes.splice(index, count, replacement as Node);
        }
      }
      instance = instance.parent;
    }
  }

  private $toggleCond(c: CondBinding): void {
    const show = c.condition[0](this.$conditionResolver(), c.scope);
    if (show) {
      if (c.instance) {
        this.$updateInstance(c.instance);
        return;
      }

      const anchor = c.anchor;
      const container = anchor?.parentNode as (ParentNode & Node) | null;
      if (!anchor || !container) return;
      const instance = this.$createBlockInstance(
        c.blockIndex,
        c.scope,
        c.owner,
        container,
      );
      if (!instance) return;
      c.instance = instance;
      const nodes = instance.nodes;
      this.$insertInstanceAfter(anchor, container, instance);
      if (nodes.length > 0 && !c.end) {
        this.$swapOwnedRange(c.owner, anchor, nodes);
        anchor.remove();
        c.anchor = null;
      }
      this.$changeStructure();
    } else if (c.instance) {
      const instance = c.instance;
      if (c.end) {
        this.$removeInstance(instance);
        c.instance = null;
        this.$changeStructure();
        return;
      }
      const block = this.$block(c.blockIndex);
      const compactOwners = !!block && this.$hasRootStructuralSlot(block);
      const first = instance.nodes[0] ?? null;
      const container = first?.parentNode as (ParentNode & Node) | null;
      if (first && container) {
        const anchor = document.createComment('');
        container.insertBefore(anchor, first);
        this.$swapOwnedRange(c.owner, instance.nodes, anchor);
        c.anchor = anchor;
      }
      this.$removeInstance(instance);
      c.instance = null;
      this.$changeStructure(compactOwners ? c.owner : undefined);
    }
  }

  // ── Value resolution ──────────────────────────────────────────

  $resolveValue(path: string, scope?: ScopeFrame): unknown {
    // Check scope frames first (repeat item variables)
    let frame = scope;
    while (frame) {
      if (path === frame.name) return frame.value;
      if (path.length > frame.name.length && path.charCodeAt(frame.name.length) === 46 && path.startsWith(frame.name)) {
        const value = dotWalk(frame.value, path, frame.name.length + 1);
        if (value !== undefined || frame.isAlias || frame.known === false) return value;
        break;
      }
      frame = frame.parent;
    }
    // Resolve against component — fast path for single-segment (no dot)
    const dot = path.indexOf('.');
    if (dot === -1) return this.$resolveComponentRoot(path);
    return dotWalk(this.$resolveComponentRoot(path.substring(0, dot)), path, dot + 1);
  }

  private $conditionResolver(): (path: string, scope?: unknown) => unknown {
    return this.$resolver ??= (path, scope) =>
      this.$resolveValue(path, scope as ScopeFrame | undefined);
  }

  /** Return whether a binding path's scope or component root is available. */
  $hasStateRoot(path: string, scope?: ScopeFrame): boolean {
    let frame = scope;
    while (frame) {
      if (path === frame.name
        || (path.length > frame.name.length
          && path.charCodeAt(frame.name.length) === 46
          && path.startsWith(frame.name))) {
        if (frame.known === false) return false;
        if (path === frame.name || frame.isAlias
          || dotWalk(frame.value, path, frame.name.length + 1) !== undefined) return true;
        break;
      }
      frame = frame.parent;
    }

    const dot = path.indexOf('.');
    const root = dot === -1 ? path : path.substring(0, dot);
    if (this.$fragmentHydrating) {
      return this.$fragmentKnownRoots?.has(root) === true ||
        (this.$templateState !== null && Object.hasOwn(this.$templateState, root));
    }
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

  private $setInstanceParent(
    instance: TemplateInstance,
    parent: TemplateInstance | undefined,
  ): void {
    instance.parent = parent;
  }

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
    inst.nodes = childNodesArray(wrapper);
    if (this.$fragmentWork) {
      // A repeat may be empty; retain one item boundary for moves and reconnect.
      const anchor = document.createComment('');
      wrapper.insertBefore(anchor, wrapper.firstChild);
      inst.nodes.unshift(anchor);
      inst.parent = parent;
      inst.callDepth = parent?.callDepth ?? 0;
      this.$fragmentWork.enqueue(inst);
      return inst;
    }
    this.$updateInstance(inst);
    if (inst.repeats.length !== 0 || inst.conds.length !== 0) {
      inst.nodes = childNodesArray(wrapper);
      this.$replaceInstanceContainer(inst, wrapper, container ?? null);
    } else if (container) {
      inst.container = container;
    }
    this.$setInstanceParent(inst, parent);
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
      instance.alive = false;
      if (!removeNodes && root.range) this.$restoreGraphMarkers(instance);
      const cleanups = instance.cleanups;
      if (cleanups) {
        for (const cleanup of cleanups) cleanup();
        cleanups.length = 0;
      }

      if (removeNodes && (!root.range || instance === root)) {
        this.$removeInstanceNodes(instance);
      }
      for (const binding of instance.conds) {
        if (binding.instance) stack.push(binding.instance);
        binding.instance = null;
      }
      for (const repeat of instance.repeats) {
        for (const child of repeat.instances) stack.push(child);
        this.$clearRepeatBinding(repeat);
      }
      if (instance.renders) {
        for (const render of instance.renders) {
          if (render.instance) stack.push(render.instance);
          if (removeNodes) {
            forgetFragmentInput(render.anchor);
            this.$releaseFragmentInput(render);
          }
          render.instance = null;
          render.scope = undefined;
          render.alias = undefined;
        }
      }
      this.$clearInstance(instance);
    }
  }

  private $restoreGraphMarkers(instance: TemplateInstance): void {
    for (const condition of instance.conds) {
      if (condition.anchor && condition.end) {
        condition.anchor.data = 'wc';
        condition.end.data = '/wc';
      }
    }
    for (const repeat of instance.repeats) {
      if (repeat.start) repeat.start.data = 'wr';
      if (repeat.end) repeat.end.data = '/wr';
      for (const child of repeat.instances) {
        const marker = child.nodes[0];
        if (marker?.nodeType === 8) (marker as Comment).data = 'wi';
      }
    }
    if (instance.renders) {
      for (const render of instance.renders) {
        if (render.alias && render.captureVersion !== undefined) {
          retainFragmentInput(render.anchor, render.alias, render.captureVersion);
          this.$releaseFragmentInput(render);
        }
        render.anchor.data = 'wf';
        render.end.data = '/wf';
      }
    }
  }

  private $removeInstanceNodes(instance: TemplateInstance): void {
    const nodes = instance.nodes;
    if (instance.range) {
      const last = nodes[nodes.length - 1];
      let node: Node | null = nodes[0] ?? null;
      while (node) {
        const next: Node | null = node.nextSibling;
        node.parentNode?.removeChild(node);
        if (node === last) break;
        node = next;
      }
      return;
    }
    for (let i = 0; i < nodes.length; i++) nodes[i].parentNode?.removeChild(nodes[i]);
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
    if (removedFrom && !removedFrom.range) {
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
    if (instance.range) {
      const last = nodes[nodes.length - 1];
      let node: Node | null = nodes[0];
      const move = (container as ParentNode & Node & {
        moveBefore?: (node: Node, before: Node | null) => void;
      }).moveBefore;
      while (node) {
        const next: Node | null = node.nextSibling;
        if (move && node.parentNode === container && node.isConnected) move.call(container, node, ref);
        else container.insertBefore(node, ref);
        if (node === last) break;
        node = next;
      }
      return last;
    }
    for (let i = 0; i < nodes.length; i++) container.insertBefore(nodes[i], ref);
    return nodes[nodes.length - 1];
  }

}
