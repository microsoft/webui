// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * WebUIElement — lightweight Web Component base class.
 *
 * Supports Shadow DOM or light DOM, SSR hydration, reactive updates, and compiled
 * SSR content is reused by matching existing DOM nodes through compiled
 * template path mapping.  Client-created components use exact childNode
 * indices from the compiled template HTML.
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
 * (`$resolveSSR`, `$findSSRText`, `$finalize`) is complete.  This is
 * critical because `$resolveSSR` uses marker pairs to skip structural
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

import { getTemplate } from './template.js';
import type {
  TemplateMeta,
  TemplateBlockMeta,
  CompiledAttrMeta,
  CompiledAttrPart,
  CompiledCondition,
  CompiledEventArgs,
  CompiledEventArg,
  TemplateNodePath,
} from './template.js';
import { hydrationStart, hydrationEnd } from './lifecycle.js';
import { getObservableNames, isAttributeProperty, syncAttrProperties } from './decorators.js';
import { syncRepeat, dotWalk } from './element/diff.js';
import {
  collectItemMarkers,
  nextElement,
  findByOrdinal,
  MARKER_COND_START,
  MARKER_COND_END,
  MARKER_REPEAT_START,
  MARKER_REPEAT_END,
} from './element/markers.js';
import {
  injectModuleStyle,
} from './element/styles.js';
import {
  ATTR_KIND_BOOLEAN,
  ATTR_KIND_COMPLEX,
  ATTR_KIND_TEMPLATE,
} from './element/types.js';
import { getTemplateAttributeMap, getTemplateRootSet } from './template-roots.js';
import type {
  AttrBinding,
  CondBinding,
  RepeatBinding,
  RepeatItemInstance,
  ScopeFrame,
  TemplateInstance,
  TextBinding,
} from './element/types.js';

// ── Caches ──────────────────────────────────────────────────────

/** Parsed template cache — cloneNode(true) is faster than re-parsing. */
const templateCache = new WeakMap<TemplateBlockMeta, DocumentFragment>();

/** Parsed template DOM for SSR path mapping, keyed by TemplateBlockMeta. */
const templateDOMCache = new WeakMap<TemplateBlockMeta, Element>();

/** Cached root tag name extracted from meta.h before it's released. */
const rootTagCache = new WeakMap<TemplateBlockMeta, string | null>();

/** Pre-computed ordinals for template nodes: childIndex → [nodeType, ordinal].
 *  Avoids re-counting element/text siblings on every $resolveSSR call. */
const tplOrdinalCache = new WeakMap<Node, Map<number, [nodeType: number, ordinal: number]>>();

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
const EMPTY_SET: ReadonlySet<string> = Object.freeze(new Set<string>()) as ReadonlySet<string>;
const EMPTY_ATTR_MAP: ReadonlyMap<string, string> = Object.freeze(new Map<string, string>()) as ReadonlyMap<string, string>;
const WEBUI_SET_STATE_KEY = Symbol.for('microsoft.webui.setStateKey');

const templateAttributeMaps = new WeakMap<Function, ReadonlyMap<string, string>>();
const templateRootSets = new WeakMap<Function, ReadonlySet<string>>();

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

function installTemplateObservedAttributes(ctor: TemplateObservedConstructor, tagName: string): void {
  const meta = getTemplate(tagName);
  if (!meta) return;

  const attrMap = getTemplateAttributeMap(meta);
  templateRootSets.set(ctor, getTemplateRootSet(meta));
  if (attrMap.size === 0) return;
  templateAttributeMaps.set(ctor, attrMap);

  const existing = ctor.observedAttributes ?? EMPTY_ARR;
  const merged = new Array<string>(existing.length + attrMap.size);
  let count = 0;
  for (let i = 0; i < existing.length; i++) {
    merged[count] = existing[i];
    count += 1;
  }
  for (const attrName of attrMap.keys()) {
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

function hasAuthoredMember(instance: object, key: string): boolean {
  if (Object.prototype.hasOwnProperty.call(instance, key)) return true;

  let proto = Object.getPrototypeOf(instance) as object | null;
  while (proto && proto !== CoreElement.prototype) {
    if (Object.prototype.hasOwnProperty.call(proto, key)) return true;
    proto = Object.getPrototypeOf(proto) as object | null;
  }
  return false;
}

// ═══════════════════════════════════════════════════════════════════
//  CoreElement — static rendering core (no events / refs / emit)
// ═══════════════════════════════════════════════════════════════════

export class CoreElement extends HTMLElement {
  private $root: TemplateInstance | null = null;
  private $meta?: TemplateMeta;
  private $ready = false;
  private $hydrated = false;
  private $templateState: Record<string, unknown> | null = null;
  private $dirtyPaths: Set<string> | null = null;
  private $pendingFlush = false;
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

  [WEBUI_SET_STATE_KEY](key: string, value: unknown): void {
    this.$setStateKey(key, value);
  }

  static define(tagName: string): void {
    installTemplateObservedAttributes(this as TemplateObservedConstructor, tagName);
    customElements.define(tagName, this);
  }

  // ── Lifecycle ─────────────────────────────────────────────────

  connectedCallback(): void {
    const tag = this.tagName.toLowerCase();

    if (this.$hydrated && this.$root) {
      hydrationStart();
      this.$ready = true;
      this.$update();
      hydrationEnd();
      return;
    }

    const meta = getTemplate(tag);
    if (!meta) {
      console.warn(
        `[WebUI] Template metadata for <${tag}> not found. ` +
        `Ensure the component is included in the SSR output or registered via __webui.templates.`,
      );
      return;
    }
    this.$meta = meta;
    // Custom element upgrade timing: when the HTML parser encounters the
    // opening tag, connectedCallback fires BEFORE children are parsed.
    // If the document is still loading, defer to let the parser finish.
    if (document.readyState === 'loading') {
      const handler = (): void => {
        document.removeEventListener('DOMContentLoaded', handler);
        this.$mount(meta);
      };
      document.addEventListener('DOMContentLoaded', handler);
    } else {
      // Document is already parsed — children are available
      this.$mount(meta);
    }
  }

  /** Mount the component after children are available. */
  private $mount(meta: TemplateMeta): void {
    if (this.$hydrated) return;
    hydrationStart();

    // Auto-detect shadow vs light DOM
    const hasShadow = !!this.shadowRoot;
    const wantShadow = hasShadow || !!meta.sd;

    let root: Node;
    let isSSR: boolean;
    let clientRoot: HTMLElement | null = null;

    if (hasShadow) {
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
      root = this.attachShadow({ mode: 'open' });
      isSSR = false;
    } else {
      // Light DOM client-created — populate from template (no shadow = no link issue)
      root = this;
      isSSR = false;
    }

    // Inject CSS module stylesheet after root is determined
    if (meta.sa) injectModuleStyle(meta.sa, this.shadowRoot);

    if (isSSR) {
      // Apply the same state that was used for SSR rendering
      // so client observables match the server-rendered DOM.
      this.$applySSRState();
      this.$root = this.$hydrate(root, meta, getTemplateDom(meta));

    } else {
      clientRoot = this.$createStagingRoot(meta);
      this.$root = this.$wire(clientRoot, meta);
    }

    this.$meta = meta;
    this.$hydrated = true;
    this.$ready = true;
    syncAttrProperties(this, this.constructor as Function);

    // Client-created components: flush current attr/observable values
    // into the freshly-wired template DOM. Call $updateInstance directly
    // to avoid the $update() path-index build — it will be lazy-built
    // on the first reactive change instead.
    if (!isSSR && clientRoot) {
      this.$updateInstance(this.$root);
      if (this.$root.repeats.length !== 0 || this.$root.conds.length !== 0) {
        this.$root.nodes = childNodesArray(clientRoot);
        this.$releaseStagingRepeatContainers(this.$root, clientRoot);
      }
      this.$appendStagedChildren(root, clientRoot);
    }

    hydrationEnd();
  }

  disconnectedCallback(): void {
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
    if (!this.$root) return;
    this.$teardown(this.$root);
    this.$root = null;
    this.$pathIndex = undefined;
    this.$wildcardBindings = undefined;
    this.$dirtyPaths = null;
    this.$pendingFlush = false;
    this.$ready = false;
  }

  /** Break all DOM references held by a binding instance and its nested blocks. */
  private $teardown(instance: TemplateInstance): void {
    for (const c of instance.conds) {
      if (c.instance) this.$teardown(c.instance);
      c.instance = null;
    }
    for (const r of instance.repeats) {
      for (const item of r.instances) this.$teardown(item.instance);
      r.instances.length = 0;
      r.container = null;
      r.start = null;
      r.end = null;
    }
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
    const property = this.$templateAttributeMap().get(name);
    if (property && this.$usesTemplateState(property)) {
      this.$setTemplateState(property, newValue);
    }
  }

  /** Populate component state from server or router state.
   *
   * Decorated properties are set through their reactive setters. Template-only
   * bindings are stored internally so app code does not need public
   * `@observable` fields just to receive server state.
   */
  setState(state: Record<string, unknown>): void {
    const keys = Object.keys(state);
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      this.$setStateKey(key, state[key]);
    }
    this.$flushUpdates();
  }

  protected $observableNames(): Set<string> {
    return getObservableNames(this.constructor as Function);
  }

  protected $shouldApplySSRState(key: string): boolean {
    return !isAttributeProperty(this.constructor as Function, key);
  }

  protected $shouldApplyTemplateStateFromSSR(_key: string): boolean {
    return true;
  }

  protected $setTemplateState(key: string, value: unknown): void {
    if (this.$writeTemplateState(key, value)) {
      this.$update(key);
    }
  }

  private $writeTemplateState(key: string, value: unknown): boolean {
    if (!this.$templateState) {
      this.$templateState = Object.create(null) as Record<string, unknown>;
    }
    if (Object.is(this.$templateState[key], value)) return false;
    this.$templateState[key] = value;
    return true;
  }

  private $templateStateNames(): ReadonlySet<string> {
    const fromCtor = templateRootSets.get(this.constructor as Function);
    if (fromCtor) return fromCtor;
    if (this.$meta) return getTemplateRootSet(this.$meta);
    const tagName = this.tagName;
    if (!tagName) return EMPTY_SET;
    const meta = getTemplate(tagName.toLowerCase());
    return meta ? getTemplateRootSet(meta) : EMPTY_SET;
  }

  private $templateAttributeMap(): ReadonlyMap<string, string> {
    const fromCtor = templateAttributeMaps.get(this.constructor as Function);
    if (fromCtor) return fromCtor;
    if (!this.$meta) {
      const meta = getTemplate(this.tagName.toLowerCase());
      return meta ? getTemplateAttributeMap(meta) : EMPTY_ATTR_MAP;
    }
    return getTemplateAttributeMap(this.$meta);
  }

  private $usesTemplateState(key: string): boolean {
    return this.$templateStateNames().has(key) && !hasAuthoredMember(this, key);
  }

  private $setStateKey(key: string, value: unknown): void {
    if (this.$observableNames().has(key)) {
      (this as Record<string, unknown>)[key] = value;
    } else if (this.$usesTemplateState(key)) {
      this.$setTemplateState(key, value);
    }
  }

  /**
   * Apply SSR state from `window.__webui.state`.
   *
   * The handler emits all SSR metadata in a single consolidated
   * `window.__webui` script block. State lives at `.state` — the same
   * props passed to the server render so observables match the DOM.
   * Only observable properties are set — unknown keys are ignored.
   *
   * Writes directly to the backing field (`_prop`) to avoid triggering
   * reactive updates before bindings are wired.
   */
  private $applySSRState(): void {
    const state = window.__webui?.state;
    if (!state || typeof state !== 'object') return;
    const observableNames = this.$observableNames();
    const keys = Object.keys(state);
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (observableNames.has(key)) {
        if (!this.$shouldApplySSRState(key)) continue;
        // Write to backing field directly — no reactive update yet
        (this as Record<string, unknown>)[`_${key}`] = state[key];
      } else if (this.$usesTemplateState(key) && this.$shouldApplyTemplateStateFromSSR(key)) {
        this.$writeTemplateState(key, state[key]);
      }
    }
  }

  /** Reactive update — called by @observable/@attr setters. */
  $update(path?: string): void {
    if (!this.$ready || !this.$root) return;

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

  /** Synchronously flush all queued path updates. Call this when you need
   *  the DOM to reflect pending property changes immediately. */
  $flushUpdates(): void {
    if (this.$pendingFlush) this.$flush();
  }

  /** Flush all queued path updates. Handles re-entrant setter calls. */
  private $flush(): void {
    if (!this.$ready || !this.$root || !this.$pathIndex) {
      this.$dirtyPaths = null;
      this.$pendingFlush = false;
      return;
    }

    while (this.$dirtyPaths && this.$dirtyPaths.size > 0) {
      // Snapshot and clear so re-entrant setters get a fresh set.
      const dirty = this.$dirtyPaths;
      this.$dirtyPaths = null;

      for (const path of dirty) {
        const entry = this.$pathIndex.get(path);
        if (entry) {
          this.$updateBindings(entry.texts, entry.attrs, entry.conds, entry.repeats);
        }
      }
      // Update wildcard bindings once per flush (not per dirty path)
      if (this.$wildcardBindings) {
        const wc = this.$wildcardBindings;
        this.$updateBindings(wc.texts, wc.attrs, wc.conds, wc.repeats);
      }
    }

    this.$pendingFlush = false;
  }

  // ── DOM resolution: client-created path ───────────────────────
  // Compiled paths are childNode indices in meta.h parsed by the browser.
  // For client-created components the DOM matches meta.h exactly.

  private $resolve(root: Node, path: TemplateNodePath, pathStart = 0): Node | null {
    let cur: Node = root;
    // When pathStart > 0, advance through the skipped segments so `cur`
    // aligns with the already-positioned SSR root.
    for (let i = 0; i < pathStart; i++) {
      const child = cur.childNodes[path[i]];
      if (!child) return null;
      cur = child;
    }
    for (let i = pathStart; i < path.length; i++) {
      const child = cur.childNodes[path[i]];
      if (!child) return null;
      cur = child;
    }
    return cur;
  }

  // ── DOM resolution: SSR hydration path ────────────────────────
  //
  // Compiled template metadata stores binding targets as paths of
  // childNode indices into the *static* template HTML (`meta.h`).
  // The SSR DOM, however, contains extra nodes injected by the server
  // for rendered structural blocks:
  //
  //   - Conditional (`<if>`) blocks: `<!--wc-->` content `<!--/wc-->`
  //   - Repeat (`<for>`) blocks: `<!--wr-->` items `<!--/wr-->`
  //
  // These extra nodes shift element/text ordinals in the SSR DOM
  // relative to the template.  For example, a template with:
  //
  //     <div class="grid"></div>          ← element ordinal 0
  //
  // becomes in SSR (when a prior <if> block renders a <p>):
  //
  //     <!--wc--><p>no results</p><!--/wc-->  ← extra content
  //     <div class="grid">...</div>           ← now element ordinal 1
  //
  // To resolve the correct element, `$resolveSSR` walks SSR children
  // in parallel with the template DOM, skipping everything between
  // structural marker pairs.  This requires closing markers to still
  // be present — marker removal is deferred to the end of $hydrate().
  //
  // pathStart: skip leading path segments for in-place block hydration.

  private $resolveSSR(ssrRoot: Node, tplRoot: Node, path: TemplateNodePath, pathStart = 0): Node | null {
    let ssr: Node = ssrRoot;
    let tpl: Node = tplRoot;

    // When pathStart > 0, ssr has already descended to the block root
    // but tpl still points at the wrapper from getTemplateDom().
    // Advance tpl through the skipped path segments to align them.
    for (let i = 0; i < pathStart; i++) {
      const tplChild = tpl.childNodes[path[i]];
      if (!tplChild) return null;
      tpl = tplChild;
    }

    for (let i = pathStart; i < path.length; i++) {
      const idx = path[i];
      const tplChild = tpl.childNodes[idx];
      if (!tplChild) return null;

      // Look up the target's nodeType and ordinal from the template.
      // getTplOrdinals maps childNode index → [nodeType, ordinal],
      // counting elements and text nodes separately (comments ignored).
      const ordinals = getTplOrdinals(tpl);
      const entry = ordinals.get(idx);
      if (!entry) return null;

      // Walk SSR children to find the Nth element/text node, skipping
      // structural block content that exists in SSR but not in meta.h.
      // See findByOrdinal() for the full algorithm and invariants.
      const [nodeType, ordinal] = entry;
      const child = findByOrdinal(ssr, nodeType, ordinal);
      if (!child) return null;
      ssr = child;
      tpl = tplChild;
    }
    return ssr;
  }

  // ── Template parsing ──────────────────────────────────────────

  private $parseTemplate(meta: TemplateBlockMeta): DocumentFragment {
    let cached = templateCache.get(meta);
    if (cached) return cached.cloneNode(true) as DocumentFragment;
    const tpl = document.createElement('template');
    tpl.innerHTML = meta.h;
    templateCache.set(meta, tpl.content);
    return tpl.content.cloneNode(true) as DocumentFragment;
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

  private $releaseStagingRepeatContainers(instance: TemplateInstance | null, stagingRoot: Node | null): void {
    if (!instance || !stagingRoot) return;
    if (instance.repeats.length === 0 && instance.conds.length === 0) return;
    const stack: TemplateInstance[] = [instance];
    while (stack.length > 0) {
      const current = stack.pop();
      if (!current) continue;
      for (let i = 0; i < current.repeats.length; i++) {
        const repeat = current.repeats[i];
        if (repeat.container === stagingRoot) repeat.container = null;
        for (let j = 0; j < repeat.instances.length; j++) {
          stack.push(repeat.instances[j].instance);
        }
      }
      for (let i = 0; i < current.conds.length; i++) {
        const child = current.conds[i].instance;
        if (child) stack.push(child);
      }
    }
  }

  // ═══════════════════════════════════════════════════════════════
  //  Client-created wiring — exact childNode index resolution
  // ═══════════════════════════════════════════════════════════════

  private $wire(root: Node, meta: TemplateBlockMeta, scope?: ScopeFrame): TemplateInstance {
    const instance: TemplateInstance = {
      scope, nodes: childNodesArray(root),
      texts: [], attrs: [], conds: [], repeats: [],
    };

    // Resolve ALL slot reference nodes BEFORE inserting any anchors.
    // Inserting comment anchors shifts childNode indices, so we must
    // capture target positions from the untouched DOM first.

    // Pre-resolve text binding slots
    const textRefs: Array<{ parent: Node; ref: Node | null; parts: CompiledAttrPart[]; raw?: boolean }> = [];
    if (meta.tx) {
      for (let i = 0; i < meta.tx.length; i++) {
        const entry = meta.tx[i];
        const [slot, parts] = entry;
        const raw = entry[2] === 1;
        const [parentPath, beforeIndex] = slot;
        const parent = parentPath.length > 0 ? this.$resolve(root, parentPath) : root;
        if (!parent || (parent.nodeType !== 1 && parent.nodeType !== 11)) continue;
        textRefs.push({ parent, ref: parent.childNodes[beforeIndex] || null, parts, raw });
      }
    }

    // Pre-resolve conditional slots
    type CondRef = { parent: Node; ref: Node | null; condition: CompiledCondition; blockIndex: number };
    const condRefs: CondRef[] = [];
    if (meta.c) {
      for (let i = 0; i < meta.c.length; i++) {
        const [condition, blockIndex, slotMeta] = meta.c[i];
        const [parentPath, beforeIndex] = slotMeta;
        const parent = parentPath.length > 0 ? this.$resolve(root, parentPath) : root;
        if (!parent || (parent.nodeType !== 1 && parent.nodeType !== 11)) continue;
        condRefs.push({ parent, ref: parent.childNodes[beforeIndex] || null, condition: condition as CompiledCondition, blockIndex });
      }
    }

    // Pre-resolve repeat slots
    type RepRef = { parent: Node; ref: Node | null; collection: string; itemVar: string; blockIndex: number };
    const repRefs: RepRef[] = [];
    if (meta.r) {
      for (let i = 0; i < meta.r.length; i++) {
        const [collection, itemVar, blockIndex, slotMeta] = meta.r[i];
        const [parentPath, beforeIndex] = slotMeta;
        const parent = parentPath.length > 0 ? this.$resolve(root, parentPath) : root;
        if (!parent || (parent.nodeType !== 1 && parent.nodeType !== 11)) continue;
        repRefs.push({ parent, ref: parent.childNodes[beforeIndex] || null, collection, itemVar, blockIndex });
      }
    }

    // Attribute bindings (no DOM mutation — safe to resolve inline)
    this.$wireAttrs(instance, meta, scope, (p) => this.$resolve(root, p));

    // Events + refs — resolve BEFORE anchors shift childNode indices.
    // Events target element nodes (not text/comment positions), but anchor
    // insertions still shift childNode indices for sibling elements.
    this.$finalize(root, meta, (r, p) => this.$resolve(r, p), scope);

    // Now insert anchors using pre-resolved references

    // Text bindings
    for (const t of textRefs) {
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
    for (const c of condRefs) {
      const anchor = document.createComment('');
      c.parent.insertBefore(anchor, c.ref);
      instance.conds.push({ condition: c.condition, blockIndex: c.blockIndex, anchor, scope, instance: null });
    }

    // Repeat bindings
    for (let i = 0; i < repRefs.length; i++) {
      const r = repRefs[i];
      const anchor = document.createComment('');
      r.parent.insertBefore(anchor, r.ref);
      const { attrMap, rootBindings } = this.$repeatMaps(r.blockIndex, r.itemVar);
      instance.repeats.push({
        markerId: i, collection: r.collection, itemVar: r.itemVar, blockIndex: r.blockIndex,
        container: r.parent as ParentNode & Node, start: anchor, end: null,
        scope, owner: instance, instances: [], rootTag: null,
        attrMap, rootBindings,
      });
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
      nodes: pathStart > 0 ? [ssrRoot] : childNodesArray(ssrRoot),
      texts: [], attrs: [], conds: [], repeats: [],
    };

    // Collect SSR markers for deferred removal.  Closing markers
    // (<!--/wc-->, <!--/wr-->) and item markers (<!--wi-->) must stay in
    // the DOM throughout the entire hydration pass so that $resolveSSR
    // and $findSSRText can correctly skip structural block content when
    // counting element/text ordinals.  All collected markers are removed
    // in a single cleanup pass after $finalize() (events + refs).
    //
    // Hydration order:  text → attrs → conditionals → repeats → events
    // All phases use $resolveSSR, so markers must survive until the end.
    const staleMarkers: Node[] = [];

    // Text bindings — find existing text nodes rendered by the server
    if (meta.tx) {
      for (let i = 0; i < meta.tx.length; i++) {
        const entry = meta.tx[i];
        const [slot, parts] = entry;
        const raw = entry[2] === 1;
        const [parentPath, beforeIndex] = slot;
        const ssrParent = this.$resolveSSR(ssrRoot, tplDom, parentPath, pathStart);
        if (!ssrParent) continue;
        const tplParent = this.$resolve(tplDom, parentPath, pathStart);
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
    this.$wireAttrs(instance, meta, scope, (p) =>
      this.$resolveSSR(ssrRoot, tplDom, p, pathStart) as Element,
    );

    // Conditional bindings — use <!--wc--> markers as anchors
    if (meta.c) {
      let lastCondMarker: Node | null = null;
      let lastCondParent: Node | null = null;
      for (let i = 0; i < meta.c.length; i++) {
        const [condition, blockIndex, slotMeta] = meta.c[i];
        const [parentPath] = slotMeta;
        const ssrParent = this.$resolveSSR(ssrRoot, tplDom, parentPath, pathStart) ?? ssrRoot;
        const blockMeta = this.$block(blockIndex);
        const shown = (condition as CompiledCondition)[0](this.$resolver, scope);
        let condInstance: TemplateInstance | null = null;

        // Reset cursor when parent changes between iterations
        if (ssrParent !== lastCondParent) {
          lastCondMarker = null;
          lastCondParent = ssrParent;
        }

        // Find the next <!--wc--> marker in ssrParent (after any previously found one)
        const marker = this.$findMarker(ssrParent, MARKER_COND_START, lastCondMarker);
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
        if (marker) lastCondMarker = marker;

        // SSR hydration: if the marker exists, the server rendered this
        // conditional — hydrate its content regardless of the current
        // condition value.  Complex properties from parent bindings may
        // not have arrived yet (the parent hydrates after children), so
        // the condition could evaluate to false even though the server
        // rendered it as true.  Trust the SSR DOM and wire it up; the
        // condition will re-evaluate correctly once all data is set.
        const ssrContentPresent = marker && blockMeta && this.$hasContentAfterMarker(condAnchor, MARKER_COND_END);
        if (blockMeta && (shown || ssrContentPresent)) {
          if (marker) {
            condInstance = this.$hydrateCondContent(condAnchor, blockMeta, scope);
          }
        }

        // Collect <!--/wc--> end marker for deferred removal.
        // Do NOT remove here — later phases (repeats, events) still need
        // intact marker pairs for $resolveSSR structural-block skipping.
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
          scope, instance: condInstance,
        });
      }
    }

    // Repeat bindings — use <!--wr--> markers as anchors, <!--wi--> for items
    if (meta.r) {
      let lastRepMarker: Node | null = null;
      let lastRepParent: Node | null = null;
      for (let i = 0; i < meta.r.length; i++) {
        const [collection, itemVar, blockIndex, slotMeta] = meta.r[i];
        const [parentPath] = slotMeta;
        const ssrParent = this.$resolveSSR(ssrRoot, tplDom, parentPath, pathStart) ?? ssrRoot;

        // Reset cursor when parent changes between iterations
        if (ssrParent !== lastRepParent) {
          lastRepMarker = null;
          lastRepParent = ssrParent;
        }

        const blockMeta = this.$block(blockIndex);
        const { attrMap, rootBindings } = this.$repeatMaps(blockIndex, itemVar);
        const blockTplDom = blockMeta ? getTemplateDom(blockMeta) : null;
        const rootTag = blockMeta && blockTplDom?.childNodes.length === 1 && blockTplDom.children.length === 1
          ? this.$rootTag(blockMeta)
          : null;
        const keyPath = Object.values(attrMap)[0];

        // Find the next <!--wr--> marker in ssrParent (after any previously found one)
        const marker = this.$findMarker(ssrParent, MARKER_REPEAT_START, lastRepMarker);
        let anchor: Comment;
        if (marker) {
          anchor = marker;
        } else {
          // No marker — insert anchor at the slot position for client-created content
          anchor = document.createComment('');
          const [, beforeIndex] = slotMeta;
          const tplParent = this.$resolve(tplDom, parentPath, pathStart);
          const staticCount = tplParent ? tplParent.childNodes.length : 0;
          const insertRef = ssrParent.childNodes[Math.min(beforeIndex ?? staticCount, ssrParent.childNodes.length)] ?? null;
          ssrParent.insertBefore(anchor, insertRef);
        }
        lastRepMarker = anchor;

        const repeatInsts: RepeatItemInstance[] = [];
        const itemsArr = this.$resolveValue(collection, scope);
        const items = Array.isArray(itemsArr) ? itemsArr as unknown[] : [];

        // Collect SSR markers — single walk captures items + end boundary
        const { items: itemMarkers, end: endMarker } = marker
          ? collectItemMarkers(anchor)
          : { items: [] as Comment[], end: null as Comment | null };

        if (blockMeta && blockTplDom && items.length > 0 && anchor.parentNode && itemMarkers.length > 0) {
          if (itemMarkers.length !== items.length) {
            console.warn(
              `[webui] hydration: repeat marker count (${itemMarkers.length}) ≠ data length (${items.length}) for "${collection}"`,
            );
          }
          const firstKey = Object.keys(attrMap)[0];
          const limit = Math.min(itemMarkers.length, items.length);
          for (let j = 0; j < limit; j++) {
            const itemValue = items[j];
            const itemScope: ScopeFrame = { name: itemVar, value: itemValue, parent: scope };

            if (rootTag) {
              const itemEl = nextElement(itemMarkers[j]);
              if (itemEl) {
                const key = firstKey !== undefined
                  ? itemEl.getAttribute(firstKey)
                  : String(j);
                const childInstance = this.$hydrate(itemEl, blockMeta, blockTplDom, itemScope, 1);
                repeatInsts.push({ key, value: itemValue, instance: childInstance });
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
              inst.nodes = childNodesArray(wrapper);
              let afterNode: Node = itemMarkers[j];
              for (let nodeIndex = 0; nodeIndex < inst.nodes.length; nodeIndex++) {
                const node = inst.nodes[nodeIndex];
                itemParent?.insertBefore(node, afterNode.nextSibling);
                afterNode = node;
              }
              const key = keyPath ? String(dotWalk(itemValue, keyPath, 0) ?? '') : null;
              repeatInsts.push({ key, value: itemValue, instance: inst });
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

        instance.repeats.push({
          markerId: i, collection, itemVar, blockIndex,
          container: (anchor.parentNode ?? ssrRoot) as ParentNode & Node,
          start: anchor, end: null,
          scope, owner: instance, instances: repeatInsts, rootTag,
          attrMap, rootBindings, synced: true,
        });
      }
    }

    // Events + refs — this is the last phase that uses $resolveSSR.
    this.$finalize(ssrRoot, meta, (r, p) => this.$resolveSSR(r, tplDom, p, pathStart), scope);

    // All path-based resolution is complete. Remove the SSR markers that
    // were kept alive for structural-block skipping.  Start markers
    // (<!--wc-->, <!--wr-->) are intentionally NOT collected — they
    // remain as runtime anchors for conditional/repeat toggling.
    for (let i = 0; i < staleMarkers.length; i++) {
      staleMarkers[i].parentNode?.removeChild(staleMarkers[i]);
    }

    return instance;
  }

  // ── SSR helpers ───────────────────────────────────────────────

  /** Collect sibling nodes between a start marker and an end marker comment. */
  private $collectBetween(start: Comment, endData: string): Node[] {
    const nodes: Node[] = [];
    let node: Node | null = start.nextSibling;
    while (node) {
      if (node.nodeType === 8 && (node as Comment).data === endData) break;
      nodes.push(node);
      node = node.nextSibling;
    }
    return nodes;
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
    if (rootTag && tplDom.children.length === 1) {
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
    // Multi-root or text-only conditional: collect nodes between <!--wc--> and <!--/wc-->
    const condNodes = this.$collectBetween(condAnchor, MARKER_COND_END);
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
    // Same as above — trust SSR DOM, skip binding evaluation.
    return inst;
  }

  /**
   * Find the next marker comment with the given data among a parent's children.
   * Starts searching from `after` (exclusive) if provided, or from firstChild.
   */
  private $findMarker(parent: Node, data: string, after?: Node | null): Comment | null {
    let child = after ? after.nextSibling : parent.firstChild;
    while (child) {
      if (child.nodeType === 8 && (child as Comment).data === data) {
        return child as Comment;
      }
      child = child.nextSibling;
    }
    return null;
  }

  /**
   * Check whether there is non-marker content between a conditional
   * start anchor and its closing marker.  Used during SSR hydration to
   * detect server-rendered conditional content even when the runtime
   * condition value has not been set yet (e.g. complex property from a
   * parent repeat binding that hydrates after its children).
   */
  private $hasContentAfterMarker(anchor: Comment, endData: string): boolean {
    let sibling = anchor.nextSibling;
    while (sibling) {
      if (sibling.nodeType === 8 && (sibling as Comment).data === endData) {
        return false; // reached end marker with no content in between
      }
      return true; // any non-end-marker node = content present
    }
    return false;
  }

  /**
   * Find existing SSR text node by mapping template text-node ordinal.
   *
   * Similar to `$resolveSSR`, the SSR DOM may contain extra text nodes
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
    // content — same algorithm as $resolveSSR (see findByOrdinal).
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
    resolve: (path: TemplateNodePath) => Node | null,
  ): void {
    if (!meta.a || !meta.ag) return;
    for (let g = 0; g < meta.ag.length; g++) {
      const [targetPath, start, count] = meta.ag[g];
      const el = resolve(targetPath);
      if (!el || el.nodeType !== 1) continue;
      for (let j = 0; j < count; j++) {
        const entry = meta.a[start + j];
        if (entry) instance.attrs.push(this.$makeAttr(el as Element, entry, scope));
      }
    }
  }

  /**
   * Hook for wiring interactivity (events + refs). The static rendering core
   * does nothing here; the interactive {@link WebUIElement} subclass overrides
   * it. Auto-elements — which can never carry event handlers — use this empty
   * core hook and tree-shake every event/ref helper away.
   */
  protected $finalize(
    _root: Node,
    _meta: TemplateBlockMeta,
    _resolver: (root: Node, path: TemplateNodePath) => Node | null,
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

  /** Build attrMap and rootBindings for a repeat block. */
  private $repeatMaps(blockIndex: number, itemVar: string): {
    attrMap: Record<string, string>;
    rootBindings: CompiledAttrMeta[];
  } {
    const attrMap: Record<string, string> = {};
    const rootBindings: CompiledAttrMeta[] = [];
    const bm = this.$block(blockIndex);
    if (bm?.a && bm.ag) {
      for (let g = 0; g < bm.ag.length; g++) {
        const [tp, s, c] = bm.ag[g];
        // tp.length === 0 means root of the block container;
        // tp = [0] means the first (and typically only) child element,
        // which IS the repeated root element in blocks like <todo-item>.
        const isRoot = tp.length === 0 || (tp.length === 1 && tp[0] === 0);
        if (isRoot) {
          for (let j = 0; j < c; j++) {
            const entry = bm.a[s + j];
            if (entry) {
              rootBindings.push(entry);
              if (entry[1] === 0 || entry[1] === 3) {
                const dp = this.$singleDynamic(
                  entry[1] === 3 ? (entry[2] as CompiledAttrPart[]) : [[entry[2] as string]],
                );
                // Only use item-scoped paths as keys; outer-scope bindings
                // (e.g. group.name inside <for each="opt in ...">) would
                // resolve to the same value for every item and break keying.
                if (dp && dp.path.startsWith(itemVar + '.')) {
                  attrMap[entry[0]] = dp.path.slice(itemVar.length + 1);
                }
              }
            }
          }
        }
      }
    }
    return { attrMap, rootBindings };
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
          visit(rep.instances[i].instance);
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
  ): void {
    for (let i = 0; i < texts.length; i++) this.$patchText(texts[i]);
    for (let i = 0; i < attrs.length; i++) this.$patchAttr(attrs[i]);
    for (let i = 0; i < conds.length; i++) this.$toggleCond(conds[i]);
    for (let i = 0; i < repeats.length; i++) {
      syncRepeat(this, repeats[i]);
    }
  }

  $updateInstance(instance: TemplateInstance): void {
    this.$updateBindings(instance.texts, instance.attrs, instance.conds, instance.repeats);
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
        const target = el as unknown as Record<string | symbol, unknown>;
        const setStateKey = target[WEBUI_SET_STATE_KEY];
        if (typeof setStateKey === 'function') {
          (setStateKey as (key: string, value: unknown) => void).call(el, b.name, v);
          const flush = target['$flushUpdates'];
          if (typeof flush === 'function') (flush as () => void).call(el);
        } else {
          target[b.name] = v;
        }
        break;
      }
      case ATTR_KIND_BOOLEAN: {
        const show = b.condition![0](this.$resolver, b.scope);
        if (show) el.setAttribute(b.name, '');
        else el.removeAttribute(b.name);
        // Form control properties must be set via DOM property, not attribute
        if (b.name === 'checked' || b.name === 'selected' || b.name === 'disabled') {
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
        if (b.name === 'checked' || b.name === 'selected') {
          (el as unknown as Record<string, unknown>)[b.name] = !!v && v !== 'false' && v !== '0';
        } else if (b.name === 'value') {
          if ((el as HTMLInputElement).value !== s) (el as HTMLInputElement).value = s;
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
      if (!c.instance) {
        c.instance = this.$createBlockInstance(c.blockIndex, c.scope);
        if (c.instance) {
          const frag = document.createDocumentFragment();
          for (const n of c.instance.nodes) frag.appendChild(n);
          c.anchor.parentNode?.insertBefore(frag, c.anchor.nextSibling);
        }
      } else {
        this.$updateInstance(c.instance);
      }
    } else if (c.instance) {
      this.$removeInstance(c.instance);
      c.instance = null;
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

  private $resolveComponentRoot(root: string): unknown {
    const instance = this as Record<string, unknown>;
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

  $createBlockInstance(blockIndex: number, scope?: ScopeFrame): TemplateInstance | null {
    const bm = this.$block(blockIndex);
    if (!bm) return null;
    const wrapper = this.$createStagingRoot(bm);
    const inst = this.$wire(wrapper, bm, scope);
    inst.nodes = childNodesArray(wrapper);
    this.$updateInstance(inst);
    if (inst.repeats.length !== 0 || inst.conds.length !== 0) {
      inst.nodes = childNodesArray(wrapper);
      this.$releaseStagingRepeatContainers(inst, wrapper);
    }
    return inst;
  }

  $removeInstance(instance: TemplateInstance): void {
    for (const n of instance.nodes) n.parentNode?.removeChild(n);
    for (const c of instance.conds) {
      if (c.instance) this.$removeInstance(c.instance);
    }
    for (const r of instance.repeats) {
      for (const item of r.instances) this.$removeInstance(item.instance);
    }
  }

  $insertInstanceAfter(cursor: Node | null, container: ParentNode & Node, instance: TemplateInstance): Node | null {
    const nodes = instance.nodes;
    if (nodes.length === 0) return cursor;
    const ref = cursor ? cursor.nextSibling : container.firstChild;
    if (nodes[0] === ref) return nodes[nodes.length - 1];
    const frag = document.createDocumentFragment();
    for (let i = 0; i < nodes.length; i++) frag.appendChild(nodes[i]);
    container.insertBefore(frag, ref);
    return nodes[nodes.length - 1];
  }

  /** Extract the single dynamic path from a compiled attr parts array. */
  private $singleDynamic(parts: CompiledAttrPart[]): { path: string; prefix: string; suffix: string } | null {
    let path = ''; let prefix = ''; let suffix = ''; let seen = false;
    for (const p of parts) {
      if (typeof p === 'string') { if (seen) suffix += p; else prefix += p; continue; }
      if (seen) return null;
      path = p[0]; seen = true;
    }
    return seen ? { path, prefix, suffix } : null;
  }
}

// ═══════════════════════════════════════════════════════════════════
//  WebUIElement — interactive superset (events + refs + emit)
// ═══════════════════════════════════════════════════════════════════

/**
 * The interactive element base. Authored components extend this to gain event
 * binding (`@click`, root events), `w-ref` wiring, and `$emit`. HTML-only
 * components never reach this class: the auto-element runtime extends
 * {@link CoreElement} directly, so a purely static app tree-shakes everything
 * below out of its bundle.
 */
export class WebUIElement extends CoreElement {
  /** Dispatch a bubbling custom event. Uses composed:true when in shadow DOM. */
  $emit(name: string, detail?: unknown): boolean {
    return this.dispatchEvent(
      new CustomEvent(name, {
        bubbles: true,
        cancelable: true,
        composed: !!this.shadowRoot,
        detail,
      }),
    );
  }

  /** Wire events + root events + refs (shared by $wire and $hydrate). */
  protected override $finalize(
    root: Node,
    meta: TemplateBlockMeta,
    resolver: (root: Node, path: TemplateNodePath) => Node | null,
    scope?: ScopeFrame,
  ): void {
    this.$wireEvents(root, meta, resolver, scope);
    if ((meta as TemplateMeta).re) this.$wireRoot((meta as TemplateMeta).re!);
    this.$wireRefs(root);
  }

  /** Wire events using a resolver function (works for both client and SSR). */
  private $wireEvents(
    root: Node,
    meta: TemplateBlockMeta,
    resolver: (root: Node, path: TemplateNodePath) => Node | null,
    scope?: ScopeFrame,
  ): void {
    if (!meta.e) return;
    for (let i = 0; i < meta.e.length; i++) {
      const [eventName, handlerName, args, target] = meta.e[i];
      const el = resolver(root, target);
      if (!el || el.nodeType !== 1) continue;
      this.$addEvent(el as Element, eventName, handlerName, args, scope);
    }
  }

  /** Wire root-level events on the host element (or shadow root when present). */
  private $wireRoot(re: [string, string, CompiledEventArgs][]): void {
    const target = this.shadowRoot ?? this;
    for (let i = 0; i < re.length; i++) {
      this.$addEvent(target, re[i][0], re[i][1], re[i][2], undefined);
    }
  }

  /** Attach a single event listener. */
  private $addEvent(
    target: EventTarget,
    eventName: string,
    handlerName: string,
    args: CompiledEventArgs,
    scope?: ScopeFrame,
  ): void {
    const method = (this as Record<string, unknown>)[handlerName];
    if (typeof method !== 'function') return;
    if (args.length === 0) {
      target.addEventListener(eventName, () => {
        (method as Function).call(this);
      });
      return;
    }
    if (args.length === 1 && args[0][0] === 'e') {
      target.addEventListener(eventName, (event) => {
        (method as Function).call(this, event);
      });
      return;
    }
    target.addEventListener(eventName, (event) => {
      (method as Function).apply(this, this.$resolveEventArgs(args, event, scope));
    });
  }

  private $resolveEventArgs(args: CompiledEventArgs, event: Event, scope?: ScopeFrame): unknown[] {
    const resolved: unknown[] = [];
    for (let i = 0; i < args.length; i++) {
      resolved.push(this.$resolveEventArg(args[i], event, scope));
    }
    return resolved;
  }

  private $resolveEventArg(arg: CompiledEventArg, event: Event, scope?: ScopeFrame): unknown {
    switch (arg[0]) {
      case 'e': return event;
      case 'p': return this.$resolveValue(arg[1], scope);
      case 's': return arg[1];
      case 'n': return arg[1];
      case 'b': return !!arg[1];
      case 'z': return null;
    }
  }

  /** Find w-ref attributes and assign to component properties. */
  private $wireRefs(root: Node): void {
    if (root.nodeType !== 1 && root.nodeType !== 11) return;
    const refs = (root as Element).querySelectorAll('[w-ref]');
    for (let i = 0; i < refs.length; i++) {
      const raw = refs[i].getAttribute('w-ref');
      if (!raw || raw.charCodeAt(0) !== 123) continue;
      const name = raw.slice(1, -1);
      if (name) (this as Record<string, unknown>)[name] = refs[i];
    }
  }
}
