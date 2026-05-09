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
  TemplateNodePath,
} from './template.js';
import { hydrationStart, hydrationEnd } from './lifecycle.js';
import { getObservableNames } from './decorators.js';
import { syncRepeat, dotWalk } from './element/diff.js';
import {
  collectItemMarkers,
  nextElement,
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
import type {
  AttrBinding,
  CondBinding,
  RepeatBinding,
  RepeatItemInstance,
  ScopeFrame,
  TemplateInstance,
  TextBinding,
} from './element/types.js';
import {
  childNodesArray,
  getTemplateDom,
  resolve,
  resolveSSR,
  findSSRText,
  findMarker,
  collectBetween,
  hasContentAfterMarker,
  rootTag,
} from './element/resolve.js';
import type { EventStateHost } from './element/events.js';
import {
  finalize as finalizeEvents,
  removeAllEvents,
  removeDirectEventsForInstances,
} from './element/events.js';

// ── Caches ──────────────────────────────────────────────────────

/** Parsed template cache — cloneNode(true) is faster than re-parsing. */
const templateCache = new WeakMap<TemplateBlockMeta, DocumentFragment>();

// ═══════════════════════════════════════════════════════════════════
//  WebUIElement
// ═══════════════════════════════════════════════════════════════════

export class WebUIElement extends HTMLElement {
  private $root: TemplateInstance | null = null;
  private $meta?: TemplateMeta;
  private $ready = false;
  private $hydrated = false;
  private $dirtyPaths: Set<string> | null = null;
  private $pendingFlush = false;
  /** Cached condition resolver — allocated lazily on first boolean condition. */
  private $resolverFn: ((p: string, s?: unknown) => unknown) | null = null;
  private get $resolver(): (p: string, s?: unknown) => unknown {
    return this.$resolverFn ?? (this.$resolverFn = (p, s) =>
      this.$resolveValue(p, s as ScopeFrame | undefined));
  }
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

  static define(tagName: string): void {
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
      const fragment = this.$parseTemplate(meta);
      root.appendChild(fragment);
      isSSR = false;
    } else {
      // Light DOM client-created — populate from template (no shadow = no link issue)
      const fragment = this.$parseTemplate(meta);
      this.appendChild(fragment);
      root = this;
      isSSR = false;
    }

    // Inject CSS module stylesheet after root is determined
    if (meta.sa) injectModuleStyle(meta.sa, this.shadowRoot);

    if (isSSR) {
      // Apply the same state that was used for SSR rendering
      // so client observables match the server-rendered DOM.
      this.$hydrateState();
      this.$root = this.$hydrate(root, meta, getTemplateDom(meta));

    } else {
      this.$root = this.$wire(root, meta);
    }

    this.$meta = meta;
    this.$hydrated = true;
    this.$ready = true;

    // Client-created components: flush current attr/observable values
    // into the freshly-wired template DOM. Call $updateInstance directly
    // to avoid the $update() path-index build — it will be lazy-built
    // on the first reactive change instead.
    if (!isSSR) {
      this.$updateInstance(this.$root);
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
    this.$disposeInstance(this.$root, false, false);
    removeAllEvents(this as unknown as EventStateHost);
    this.$root = null;
    this.$meta = undefined;
    if (this.$pathIndex) {
      this.$pathIndex.clear();
      this.$pathIndex = undefined;
    }
    this.$wildcardBindings = undefined;
    if (this.$dirtyPaths) {
      this.$dirtyPaths.clear();
      this.$dirtyPaths = null;
    }
    this.$pendingFlush = false;
    this.$ready = false;
    this.$hydrated = false;
  }

  /** Break all DOM references held by a binding instance and its nested blocks. */
  private $disposeInstance(root: TemplateInstance, removeDom: boolean, removeDirectEvents: boolean = true): void {
    const stack: TemplateInstance[] = [root];
    for (let i = 0; i < stack.length; i++) {
      const instance = stack[i];
      if (removeDom) {
        for (let n = 0; n < instance.nodes.length; n++) {
          instance.nodes[n].parentNode?.removeChild(instance.nodes[n]);
        }
      }
      for (let c = 0; c < instance.conds.length; c++) {
        const child = instance.conds[c].instance;
        if (child) {
          stack.push(child);
          instance.conds[c].instance = null;
        }
      }
      for (let r = 0; r < instance.repeats.length; r++) {
        const rep = instance.repeats[r];
        for (let item = 0; item < rep.instances.length; item++) {
          stack.push(rep.instances[item].instance);
        }
        rep.instances.length = 0;
        rep.container = null;
        rep.start = null;
        rep.end = null;
      }
      instance.scope = undefined;
      for (let ref = 0; ref < instance.refs.length; ref++) {
        const binding = instance.refs[ref];
        if ((this as Record<string, unknown>)[binding.name] === binding.node) {
          (this as Record<string, unknown>)[binding.name] = undefined;
        }
      }
      instance.nodes.length = 0;
      instance.texts.length = 0;
      instance.attrs.length = 0;
      instance.conds.length = 0;
      instance.repeats.length = 0;
      instance.refs.length = 0;
    }
    if (removeDirectEvents) removeDirectEventsForInstances(this as unknown as EventStateHost, stack);
  }

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

  /** Populate @observable properties from server or router state.
   *
   * Each property is set through its reactive setter, which coalesces
   * updates into a single pending microtask. We then synchronously
   * flush those pending path updates so the DOM is current before any
   * view-transition snapshot captures it.
   */
  setState(state: Record<string, unknown>): void {
    const names = getObservableNames(this.constructor as Function);
    const keys = Object.keys(state);
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (names.has(key)) {
        (this as Record<string, unknown>)[key] = state[key];
      }
    }
    this.$flushUpdates();
  }

  /**
   * Hydrate SSR state into @observable backing fields.
   *
   * State is always delivered on `window.__webui.chain[0].state` — for
   * router apps this is the matched root route, for non-router apps the
   * server emits a state-only envelope at chain[0]. Either way the chain
   * (and the state with it) is freed after initial hydration.
   *
   * Each component picks only keys matching its own `@observable` names.
   * Writes directly to the backing field (`_prop`) to avoid triggering
   * reactive updates before bindings are wired.
   */
  private $hydrateState(): void {
    const chain = window.__webui?.chain;
    const state = Array.isArray(chain) ? chain[0]?.state : undefined;
    if (!state || typeof state !== 'object') return;
    const names = getObservableNames(this.constructor as Function);
    for (const key of Object.keys(state)) {
      if (names.has(key)) {
        // Write to backing field directly — no reactive update yet
        (this as Record<string, unknown>)[`_${key}`] = state[key];
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

  // ── Template parsing ──────────────────────────────────────────

  private $parseTemplate(meta: TemplateBlockMeta): DocumentFragment {
    let cached = templateCache.get(meta);
    if (cached) return cached.cloneNode(true) as DocumentFragment;
    const tpl = document.createElement('template');
    tpl.innerHTML = meta.h;
    templateCache.set(meta, tpl.content);
    return tpl.content.cloneNode(true) as DocumentFragment;
  }

  // ═══════════════════════════════════════════════════════════════
  //  Client-created wiring — exact childNode index resolution
  // ═══════════════════════════════════════════════════════════════

  private $wire(root: Node, meta: TemplateBlockMeta, scope?: ScopeFrame): TemplateInstance {
    const instance: TemplateInstance = {
      scope, nodes: childNodesArray(root),
      texts: [], attrs: [], conds: [], repeats: [], refs: [],
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
        const parent = parentPath.length > 0 ? resolve(root, parentPath) : root;
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
        const parent = parentPath.length > 0 ? resolve(root, parentPath) : root;
        if (!parent || (parent.nodeType !== 1 && parent.nodeType !== 11)) continue;
        condRefs.push({ parent, ref: parent.childNodes[beforeIndex] || null, condition, blockIndex });
      }
    }

    // Pre-resolve repeat slots
    type RepRef = { parent: Node; ref: Node | null; collection: string; itemVar: string; blockIndex: number };
    const repRefs: RepRef[] = [];
    if (meta.r) {
      for (let i = 0; i < meta.r.length; i++) {
        const [collection, itemVar, blockIndex, slotMeta] = meta.r[i];
        const [parentPath, beforeIndex] = slotMeta;
        const parent = parentPath.length > 0 ? resolve(root, parentPath) : root;
        if (!parent || (parent.nodeType !== 1 && parent.nodeType !== 11)) continue;
        repRefs.push({ parent, ref: parent.childNodes[beforeIndex] || null, collection, itemVar, blockIndex });
      }
    }

    // Attribute bindings (no DOM mutation — safe to resolve inline)
    this.$wireAttrs(instance, meta, scope, (p) => resolve(root, p));

    // Events + refs — resolve BEFORE anchors shift childNode indices.
    // Events target element nodes (not text/comment positions), but anchor
    // insertions still shift childNode indices for sibling elements.
    this.$finalize(root, meta, (r, p) => resolve(r, p), instance);

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
      const { attrMap, rootBindings, keyPath } = this.$repeatMaps(r.blockIndex, r.itemVar);
      instance.repeats.push({
        markerId: i, collection: r.collection, itemVar: r.itemVar, blockIndex: r.blockIndex,
        container: r.parent as ParentNode & Node, start: anchor, end: null,
        scope, owner: instance, instances: [], rootTag: null,
        attrMap, keyPath, rootBindings,
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
      texts: [], attrs: [], conds: [], repeats: [], refs: [],
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
        const ssrParent = resolveSSR(ssrRoot, tplDom, parentPath, pathStart);
        if (!ssrParent) continue;
        const tplParent = resolve(tplDom, parentPath, pathStart);
        if (!tplParent) continue;
        if (raw) {
          const rawParent = ssrParent as Element;
          const textNode = document.createTextNode('');
          instance.texts.push({ node: textNode, parts, scope, raw: true, rawParent });
        } else {
          const textNode = findSSRText(ssrParent, tplParent, beforeIndex);
          if (textNode) instance.texts.push({ node: textNode, parts, scope });
        }
      }
    }

    // Attribute bindings
    this.$wireAttrs(instance, meta, scope, (p) =>
      resolveSSR(ssrRoot, tplDom, p, pathStart) as Element,
    );

    // Conditional bindings — use <!--wc--> markers as anchors
    if (meta.c) {
      let lastCondMarker: Node | null = null;
      let lastCondParent: Node | null = null;
      for (let i = 0; i < meta.c.length; i++) {
        const [condition, blockIndex, slotMeta] = meta.c[i];
        const [parentPath] = slotMeta;
        const ssrParent = resolveSSR(ssrRoot, tplDom, parentPath, pathStart) ?? ssrRoot;
        const blockMeta = this.$block(blockIndex);
        const shown = condition[0](this.$resolver, scope);
        let condInstance: TemplateInstance | null = null;

        // Reset cursor when parent changes between iterations
        if (ssrParent !== lastCondParent) {
          lastCondMarker = null;
          lastCondParent = ssrParent;
        }

        // Find the next <!--wc--> marker in ssrParent (after any previously found one)
        const marker = findMarker(ssrParent, MARKER_COND_START, lastCondMarker);
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
        const ssrContentPresent = marker && blockMeta && hasContentAfterMarker(condAnchor, MARKER_COND_END);
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
          condition, blockIndex,
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
        const ssrParent = resolveSSR(ssrRoot, tplDom, parentPath, pathStart) ?? ssrRoot;

        // Reset cursor when parent changes between iterations
        if (ssrParent !== lastRepParent) {
          lastRepMarker = null;
          lastRepParent = ssrParent;
        }

        const blockMeta = this.$block(blockIndex);
        const { attrMap, rootBindings, keyPath } = this.$repeatMaps(blockIndex, itemVar);
        const blockRootTag = blockMeta ? rootTag(blockMeta) : null;

        // Find the next <!--wr--> marker in ssrParent (after any previously found one)
        const marker = findMarker(ssrParent, MARKER_REPEAT_START, lastRepMarker);
        let anchor: Comment;
        if (marker) {
          anchor = marker;
        } else {
          // No marker — insert anchor at the slot position for client-created content
          anchor = document.createComment('');
          const [, beforeIndex] = slotMeta;
          const tplParent = resolve(tplDom, parentPath, pathStart);
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

        if (blockMeta && items.length > 0 && anchor.parentNode && itemMarkers.length > 0) {
          if (itemMarkers.length !== items.length) {
            console.warn(
              `[webui] hydration: repeat marker count (${itemMarkers.length}) ≠ data length (${items.length}) for "${collection}"`,
            );
          }
          const firstKey = Object.keys(attrMap)[0];
          const blockTplDom = getTemplateDom(blockMeta);

          const limit = Math.min(itemMarkers.length, items.length);
          for (let j = 0; j < limit; j++) {
            const itemValue = items[j];
            const itemScope: ScopeFrame = { name: itemVar, value: itemValue, parent: scope };

            if (blockRootTag) {
              const itemEl = nextElement(itemMarkers[j]);
              if (itemEl) {
                const key = firstKey !== undefined
                  ? itemEl.getAttribute(firstKey)
                  : String(j);
                const childInstance = this.$hydrate(itemEl, blockMeta, blockTplDom, itemScope, 1);
                repeatInsts.push({ key, value: itemValue, instance: childInstance });
              }
            } else {
              // Text-only repeat item — wire nested conditionals from markers
              const inst: TemplateInstance = {
                scope: itemScope, nodes: [],
                texts: [],
                attrs: [],
                conds: [],
                repeats: [],
                refs: [],
              };

              if (blockMeta.c) {
                // Walk between this <!--wi--> and the next boundary to find <!--wc--> markers
                let cursor: Node | null = itemMarkers[j].nextSibling;
                const nextBound = j + 1 < itemMarkers.length ? itemMarkers[j + 1] : endMarker;
                const itemParent = itemMarkers[j].parentNode;

                for (let ci = 0; ci < blockMeta.c.length; ci++) {
                  const [condCond, condBlockIndex] = blockMeta.c[ci];

                  // Find <!--wc--> within this item's range
                  let condAnchor: Comment | null = null;
                  while (cursor && cursor !== nextBound) {
                    if (cursor.nodeType === 8 && (cursor as Comment).data === MARKER_COND_START) {
                      condAnchor = cursor as Comment;
                      cursor = cursor.nextSibling;
                      break;
                    }
                    cursor = cursor.nextSibling;
                  }
                  if (!condAnchor) {
                    condAnchor = document.createComment('');
                    if (itemParent) itemParent.insertBefore(condAnchor, cursor ?? null);
                  }

                  const condMet = condCond[0](this.$resolver, itemScope);
                  let condInstance: TemplateInstance | null = null;

                  if (condMet) {
                    const condBlockMeta = this.$block(condBlockIndex);
                    if (condBlockMeta) {
                      condInstance = this.$hydrateCondContent(condAnchor, condBlockMeta, itemScope);
                    }
                  }

                  // Remove <!--/wc--> end marker and advance cursor past it
                  const lastNode = condInstance ? condInstance.nodes[condInstance.nodes.length - 1] : condAnchor;
                  const endM = lastNode?.nextSibling;
                  if (endM && endM.nodeType === 8 && (endM as Comment).data === MARKER_COND_END) {
                    cursor = endM.nextSibling;
                    endM.parentNode?.removeChild(endM);
                  } else {
                    cursor = lastNode?.nextSibling ?? null;
                  }

                  inst.conds.push({
                    condition: condCond, blockIndex: condBlockIndex,
                    anchor: condAnchor, scope: itemScope,
                    instance: condInstance,
                  });
                }
              }

              repeatInsts.push({ key: String(j), value: itemValue, instance: inst });
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
          scope, owner: instance, instances: repeatInsts, rootTag: blockRootTag,
          attrMap, keyPath, rootBindings, synced: true,
        });
      }
    }

    // Events + refs — this is the last phase that uses $resolveSSR.
    this.$finalize(ssrRoot, meta, (r, p) => resolveSSR(r, tplDom, p, pathStart), instance);

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

  /**
   * Hydrate a conditional block's content — shared by top-level and
   * repeat-item conditional hydration paths.
   */
  private $hydrateCondContent(
    condAnchor: Comment,
    blockMeta: TemplateBlockMeta,
    scope: ScopeFrame | undefined,
  ): TemplateInstance | null {
    const tag = rootTag(blockMeta);
    const tplDom = getTemplateDom(blockMeta);
    if (tag && tplDom.children.length === 1) {
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
    const condNodes = collectBetween(condAnchor, MARKER_COND_END);
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
    // Trust SSR DOM, skip binding evaluation (same reasoning as the
    // single-root branch above — see comment there).
    return inst;
  }

  // ═══════════════════════════════════════════════════════════════
  //  Shared: attribute binding wiring
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

  /** Wire events + root events + refs (delegates to event engine). */
  private $finalize(
    root: Node,
    meta: TemplateBlockMeta,
    resolver: (root: Node, path: TemplateNodePath) => Node | null,
    owner: TemplateInstance,
  ): void {
    finalizeEvents(
      this as unknown as EventStateHost,
      root, meta, resolver, owner,
    );
  }

  /** Create an AttrBinding from compiled metadata. */
  private $makeAttr(el: Element, entry: CompiledAttrMeta, scope?: ScopeFrame): AttrBinding {
    const name = entry[0];
    const kind = entry[1];
    if (kind === ATTR_KIND_BOOLEAN) return { element: el, name, kind, condition: entry[2] as CompiledCondition, scope };
    if (kind === ATTR_KIND_TEMPLATE) return { element: el, name, kind, parts: entry[2] as CompiledAttrPart[], scope };
    return { element: el, name, kind: kind as number, path: (entry[2] as string) || '', scope };
  }

  /** Build attrMap, rootBindings, and pre-computed keyPath for a repeat block. */
  private $repeatMaps(blockIndex: number, itemVar: string): {
    attrMap: Record<string, string>;
    rootBindings: CompiledAttrMeta[];
    keyPath: string | null;
  } {
    const attrMap: Record<string, string> = {};
    const rootBindings: CompiledAttrMeta[] = [];
    let keyPath: string | null = null;
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
                  const path = dp.path.slice(itemVar.length + 1);
                  attrMap[entry[0]] = path;
                  // First binding wins as the diff key — matches insertion
                  // order semantics of the original `Object.values()[0]`.
                  if (keyPath === null) keyPath = path;
                }
              }
            }
          }
        }
      }
    }
    return { attrMap, rootBindings, keyPath };
  }

  // ═══════════════════════════════════════════════════════════════
  //  Reactive update system
  // ═══════════════════════════════════════════════════════════════

  private $buildPathIndex(): void {
    if (!this.$root) return;
    const observableNames = getObservableNames(this.constructor as Function);
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
      return observableNames.has(root) ? root : '*';
    };

    const r = this.$root;
    for (const t of r.texts) {
      if (t.parts) {
        for (const p of t.parts) {
          if (typeof p !== 'string') ensure(keyFor(p[0])).texts.push(t);
        }
      }
    }
    for (const a of r.attrs) {
      if (a.path) ensure(keyFor(a.path)).attrs.push(a);
      if (a.parts) {
        for (const p of a.parts) {
          if (typeof p !== 'string') ensure(keyFor(p[0])).attrs.push(a);
        }
      }
      if (a.condition) {
        for (const p of a.condition[1]) ensure(keyFor(p)).attrs.push(a);
      }
    }
    for (const c of r.conds) {
      for (const p of c.condition[1]) ensure(keyFor(p)).conds.push(c);
    }
    for (const rep of r.repeats) {
      ensure(keyFor(rep.collection)).repeats.push(rep);
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
        (el as unknown as Record<string, unknown>)[b.name] = v;
        // If the target is a WebUIElement, flush its pending updates
        // synchronously so child <for> loops re-render immediately.
        // Without this, the child's microtask-coalesced update runs
        // too late for view transitions that snapshot the DOM.
        const flush = (el as unknown as Record<string, unknown>)['$flushUpdates'];
        if (typeof flush === 'function') (flush as () => void).call(el);
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
      }
      if (c.instance) this.$updateInstance(c.instance);
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
    if (dot === -1) return (this as Record<string, unknown>)[path];
    return dotWalk((this as Record<string, unknown>)[path.substring(0, dot)], path, dot + 1);
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
    const frag = this.$parseTemplate(bm);
    const wrapper = document.createElement('div');
    wrapper.appendChild(frag);
    const inst = this.$wire(wrapper, bm, scope);
    inst.nodes = childNodesArray(wrapper);
    return inst;
  }

  $removeInstance(instance: TemplateInstance): void {
    this.$disposeInstance(instance, true);
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
