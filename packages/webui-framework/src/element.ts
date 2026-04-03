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
 * ## Comment anchors
 *
 * The framework inserts empty comment nodes (`document.createComment('')`)
 * as stable DOM anchors for conditional (`<if>`) and repeat (`<for>`) blocks.
 * This framework hydrates compiled templates against real DOM, so it needs
 * lightweight markers to know WHERE to insert or remove dynamic content.
 *
 * - **Condition anchors** mark the insertion point for `<if>` block content.
 *   When the condition becomes true, nodes are inserted after the anchor;
 *   when false, they are removed.  The anchor itself stays in the DOM.
 *
 * - **Repeat anchors** mark the start of a `<for>` block's item list.
 *   New items are inserted after the anchor; the keyed diff algorithm
 *   uses `$insertInstanceAfter` to reorder items relative to this anchor.
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
import { syncRepeat } from './element/diff.js';
import {
  injectModuleStyle,
} from './element/styles.js';
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

/** Parsed template DOM for SSR path mapping, keyed by meta.h string. */
const templateDOMCache = new Map<string, Element>();

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
  let cached = templateDOMCache.get(meta.h);
  if (cached) return cached;
  const div = document.createElement('div');
  div.innerHTML = meta.h;
  templateDOMCache.set(meta.h, div);
  return div;
}

/** Walk a dotted path from a start offset without allocating. */
function dotWalk(cursor: unknown, path: string, from: number): unknown {
  let start = from;
  for (let i = from; i <= path.length; i++) {
    if (i === path.length || path.charCodeAt(i) === 46) {
      if (cursor == null || typeof cursor !== 'object') return undefined;
      cursor = (cursor as Record<string, unknown>)[path.substring(start, i)];
      start = i + 1;
    }
  }
  return cursor;
}

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
  /** Cached condition resolver — avoids allocating a closure per evaluation. */
  private $resolver = (p: string, s?: unknown): unknown => this.$resolveValue(p, s as ScopeFrame | undefined);
  private $pathIndex?: Map<string, {
    texts: TextBinding[];
    attrs: AttrBinding[];
    conds: CondBinding[];
    repeats: RepeatBinding[];
  }>;

  static define(tagName: string): void {
    customElements.define(tagName, this);
  }

  // ── Lifecycle ─────────────────────────────────────────────────

  connectedCallback(): void {
    const tag = this.tagName.toLowerCase();

    if (this.$hydrated && this.$root) {
      hydrationStart(tag);
      this.$ready = true;
      this.$update();
      hydrationEnd(tag);
      return;
    }

    const meta = getTemplate(tag);
    if (!meta) {
      if (typeof console !== 'undefined' && console.warn) {
        console.warn(
          `[WebUI] Template metadata for <${tag}> not found. ` +
          `Ensure the component is included in the SSR output or registered via __webui_templates.`,
        );
      }
      return;
    }
    this.$meta = meta;

    // Custom element upgrade timing: when the HTML parser encounters the
    // opening tag, connectedCallback fires BEFORE children are parsed.
    // If the document is still loading, defer to let the parser finish.
    if (document.readyState === 'loading') {
      const handler = (): void => {
        document.removeEventListener('DOMContentLoaded', handler);
        this.$mount(meta, tag);
      };
      document.addEventListener('DOMContentLoaded', handler);
    } else {
      // Document is already parsed — children are available
      this.$mount(meta, tag);
    }
  }

  /** Mount the component after children are available. */
  private $mount(meta: TemplateMeta, tag: string): void {
    if (this.$hydrated) return;
    hydrationStart(tag);

    // Auto-detect shadow vs light DOM
    const hasShadow = !!this.shadowRoot;
    const wantShadow = hasShadow || !!meta.sd || !!window.__webui_shadow;

    let root: Node;
    let isSSR: boolean;

    if (hasShadow) {
      // Shadow DOM SSR — declarative shadow root already has content
      root = this.shadowRoot!;
      isSSR = true;
    } else if (this.childNodes.length > 0) {
      // SSR — element already has server-rendered children (light DOM).
      // Reuse existing DOM regardless of shadow preference.
      root = this;
      isSSR = true;
    } else if (wantShadow) {
      // Shadow DOM client-created
      root = this.attachShadow({ mode: 'open' });
      const fragment = this.$parseTemplate(meta);
      root.appendChild(fragment);
      isSSR = false;
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
      this.$applySSRState();
      this.$root = this.$hydrate(root, meta, getTemplateDom(meta));

    } else {
      this.$root = this.$wire(root, meta);
    }

    this.$meta = meta;
    this.$hydrated = true;
    this.$ready = true;

    hydrationEnd(tag);
  }

  disconnectedCallback(): void {}

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

  /** Populate @observable properties from router state. */
  setInitialState(state: Record<string, unknown>): void {
    const names = getObservableNames(this.constructor as Function);
    for (const key of Object.keys(state)) {
      if (names.has(key)) {
        (this as Record<string, unknown>)[key] = state[key];
      }
    }
  }

  /**
   * Apply SSR state from the global `window.__webui_state` object.
   *
   * Passing the same props to both server render and client
   * hydrate, this ensures component observables match the server-rendered
   * DOM. The handler emits the state as a `<script>` tag at the end of
   * the page. Only observable properties are set — unknown keys are ignored.
   *
   * Writes directly to the backing field (`_prop`) to avoid triggering
   * reactive updates before bindings are wired.
   */
  private $applySSRState(): void {
    const state = window.__webui_state;
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
    }

    this.$pendingFlush = false;
  }

  // ── DOM resolution: client-created path ───────────────────────
  // Compiled paths are childNode indices in meta.h parsed by the browser.
  // For client-created components the DOM matches meta.h exactly.

  private $resolve(root: Node, path: TemplateNodePath): Node | null {
    let cur: Node = root;
    for (let i = 0; i < path.length; i++) {
      const child = cur.childNodes[path[i]];
      if (!child) return null;
      cur = child;
    }
    return cur;
  }

  // ── DOM resolution: SSR hydration path ────────────────────────
  // SSR DOM may lack whitespace text nodes the compiled template has.
  // We walk the template DOM in parallel to translate each childNode
  // index into an element-ordinal lookup in the SSR DOM.

  private $resolveSSR(ssrRoot: Node, tplRoot: Node, path: TemplateNodePath): Node | null {
    let ssr: Node = ssrRoot;
    let tpl: Node = tplRoot;

    for (let i = 0; i < path.length; i++) {
      const idx = path[i];
      const tplChild = tpl.childNodes[idx];
      if (!tplChild) return null;

      const ordinals = getTplOrdinals(tpl);
      const entry = ordinals.get(idx);
      if (!entry) return null;

      const [nodeType, ordinal] = entry;
      let count = 0;
      let child = ssr.firstChild;
      while (child) {
        if (child.nodeType === nodeType) {
          if (count === ordinal) break;
          count++;
        }
        child = child.nextSibling;
      }
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
    this.$finalize(root, meta, (r, p) => this.$resolve(r, p));

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
  //  SSR hydration — DOM matching
  // ═══════════════════════════════════════════════════════════════

  private $hydrate(
    ssrRoot: Node,
    meta: TemplateBlockMeta,
    tplDom: Element,
    scope?: ScopeFrame,
  ): TemplateInstance {
    const instance: TemplateInstance = {
      scope, nodes: childNodesArray(ssrRoot),
      texts: [], attrs: [], conds: [], repeats: [],
    };

    // Text bindings — find existing text nodes rendered by the server
    if (meta.tx) {
      for (let i = 0; i < meta.tx.length; i++) {
        const entry = meta.tx[i];
        const [slot, parts] = entry;
        const raw = entry[2] === 1;
        const [parentPath, beforeIndex] = slot;
        // Resolve parent in SSR DOM
        const ssrParent = parentPath.length > 0
          ? this.$resolveSSR(ssrRoot, tplDom, parentPath)
          : ssrRoot;
        if (!ssrParent) continue;
        // Resolve parent in template DOM to map the text node position
        const tplParent = parentPath.length > 0
          ? this.$resolve(tplDom, parentPath)
          : tplDom;
        if (!tplParent) continue;
        if (raw) {
          // Raw binding: the SSR rendered HTML directly into the element.
          // Find the parent element and use it for innerHTML updates.
          const rawParent = ssrParent as Element;
          const textNode = document.createTextNode('');
          instance.texts.push({ node: textNode, parts, scope, raw: true, rawParent });
        } else {
          const textNode = this.$findSSRText(ssrParent, tplParent, beforeIndex);
          if (textNode) instance.texts.push({ node: textNode, parts, scope });
        }
      }
    }

    // Attribute bindings
    this.$wireAttrs(instance, meta, scope, (p) => this.$resolveSSR(ssrRoot, tplDom, p) as Element);

    // Conditional bindings
    if (meta.c) {
      for (let i = 0; i < meta.c.length; i++) {
        const [condition, blockIndex, slotMeta] = meta.c[i];
        const [parentPath] = slotMeta;
        const ssrParent = parentPath.length > 0
          ? this.$resolveSSR(ssrRoot, tplDom, parentPath)
          : ssrRoot;
        if (!ssrParent) continue;

        const blockMeta = this.$block(blockIndex);
        const shown = condition[0](this.$resolver, scope);
        let condInstance: TemplateInstance | null = null;

        // Insert anchor; if condition is true, collect existing block nodes
        const anchor = document.createComment('');
        if (shown && blockMeta) {
          const rootTag = this.$rootTag(blockMeta);

          // Use the slot's beforeIndex to skip static siblings that precede
          // the conditional block — same approach used by repeat bindings.
          const slotBeforeIndex = slotMeta[1] ?? 0;
          const tplParent = parentPath.length > 0
            ? this.$resolve(tplDom, parentPath)
            : tplDom;
          let skipCount = 0;
          if (tplParent && rootTag) {
            for (let k = 0; k < slotBeforeIndex && k < tplParent.childNodes.length; k++) {
              const n = tplParent.childNodes[k];
              if (n.nodeType === 1 && (n as Element).tagName.toLowerCase() === rootTag) {
                skipCount++;
              }
            }
          }

          const allMatches = rootTag
            ? this.$collectByTag(ssrParent, rootTag)
            : this.$collectTextChildren(ssrParent);
          const blockNodes = allMatches.slice(skipCount);
          if (blockNodes.length > 0) {
            ssrParent.insertBefore(anchor, blockNodes[0]);
            const wrapper = document.createElement('div');
            for (const n of blockNodes) wrapper.appendChild(n);
            condInstance = this.$hydrate(wrapper, blockMeta, getTemplateDom(blockMeta), scope);
            condInstance.nodes = childNodesArray(wrapper);
            // Put nodes back in place
            let after: Node = anchor;
            for (const n of condInstance.nodes) {
              ssrParent.insertBefore(n, after.nextSibling);
              after = n;
            }
            // Flush the block instance's bindings inline so nested
            // conditionals/repeats are fully evaluated during hydration.
            // This eliminates the need for a post-hydration $update() flush.
            this.$updateInstance(condInstance);
          } else {
            ssrParent.appendChild(anchor);
          }
        } else {
          ssrParent.appendChild(anchor);
        }
        instance.conds.push({ condition, blockIndex, anchor, scope, instance: condInstance });
      }
    }

    // Repeat bindings — recognize existing repeated children
    if (meta.r) {
      for (let i = 0; i < meta.r.length; i++) {
        const [collection, itemVar, blockIndex, slotMeta] = meta.r[i];
        const [parentPath] = slotMeta;
        const ssrParent = parentPath.length > 0
          ? this.$resolveSSR(ssrRoot, tplDom, parentPath)
          : ssrRoot;
        if (!ssrParent) continue;

        const blockMeta = this.$block(blockIndex);
        const { attrMap, rootBindings } = this.$repeatMaps(blockIndex, itemVar);
        const rootTag = blockMeta ? this.$rootTag(blockMeta) : null;

        // Collect existing repeated elements by tag name, starting AFTER
        // any static siblings that precede the repeat slot position.
        // The slot's beforeIndex tells us where repeats start in the template;
        // count how many same-tag static elements precede that position.
        const [, beforeIndex] = slotMeta;
        const tplParent = parentPath.length > 0
          ? this.$resolve(tplDom, parentPath) : tplDom;
        let skipCount = 0;
        if (tplParent && rootTag) {
          for (let k = 0; k < beforeIndex && k < tplParent.childNodes.length; k++) {
            const n = tplParent.childNodes[k];
            if (n.nodeType === 1 && (n as Element).tagName.toLowerCase() === rootTag) {
              skipCount++;
            }
          }
        }

        const allMatches = rootTag
          ? this.$collectByTag(ssrParent, rootTag)
          : [];
        const groups = allMatches.slice(skipCount);

        // Insert anchor before first repeat child
        const anchor = document.createComment('');
        if (groups.length > 0) {
          ssrParent.insertBefore(anchor, groups[0]);
        } else if (!rootTag) {
          // Text-only blocks: anchor goes at the repeat slot position.
          const staticCount = tplParent ? tplParent.childNodes.length : 0;
          const insertBefore = ssrParent.childNodes[staticCount] ?? null;
          ssrParent.insertBefore(anchor, insertBefore);
        } else {
          ssrParent.appendChild(anchor);
        }

        // Hydrate each existing child as a repeat instance.
        const repeatInsts: RepeatItemInstance[] = [];
        const itemsArr = this.$resolveValue(collection, scope);
        const items = Array.isArray(itemsArr) ? itemsArr as unknown[] : [];
        const blockTplDom = blockMeta ? getTemplateDom(blockMeta) : null;

        if (rootTag) {
          // Element-based blocks: hydrate by matching SSR elements by tag.
          const firstKey = Object.keys(attrMap)[0];
          const hydrateWrapper = blockMeta && blockTplDom ? document.createElement('div') : null;
          for (let j = 0; j < groups.length && j < items.length; j++) {
            const childEl = groups[j];
            const itemValue = items[j];
            const itemScope: ScopeFrame = { name: itemVar, value: itemValue, parent: scope };
            const key = firstKey !== undefined
              ? (childEl as Element).getAttribute(firstKey)
              : String(j);

            let childInstance: TemplateInstance;
            if (hydrateWrapper) {
              hydrateWrapper.appendChild(childEl);
              childInstance = this.$hydrate(hydrateWrapper, blockMeta!, blockTplDom!, itemScope);
              childInstance.nodes = [childEl]; // single root element — skip childNodesArray
              ssrParent.insertBefore(childEl, anchor.nextSibling);
            } else {
              childInstance = {
                scope: itemScope, nodes: [childEl],
                texts: EMPTY_ARR as unknown as TextBinding[],
                attrs: EMPTY_ARR as unknown as AttrBinding[],
                conds: EMPTY_ARR as unknown as CondBinding[],
                repeats: EMPTY_ARR as unknown as RepeatBinding[],
              };
            }
            repeatInsts.push({ key, value: itemValue, instance: childInstance });
          }
        } else if (blockMeta) {
          // Text-only blocks: no root element, so SSR children can't be
          // matched by tag. Walk items and for each one, evaluate its
          // conditionals to claim SSR nodes or create empty instances.
          let ssrCursor: Node | null = anchor.nextSibling;

          for (let j = 0; j < items.length; j++) {
            const itemValue = items[j];
            const itemScope: ScopeFrame = { name: itemVar, value: itemValue, parent: scope };
            const inst: TemplateInstance = {
              scope: itemScope, nodes: [],
              texts: EMPTY_ARR as unknown as TextBinding[],
              attrs: EMPTY_ARR as unknown as AttrBinding[],
              conds: [],
              repeats: EMPTY_ARR as unknown as RepeatBinding[],
            };

            if (blockMeta.c) {
              for (let ci = 0; ci < blockMeta.c.length; ci++) {
                const [condition, condBlockIndex] = blockMeta.c[ci];
                const condAnchor = document.createComment('');
                ssrParent.insertBefore(condAnchor, ssrCursor);

                const condMet = condition[0](this.$resolver, itemScope);
                let condInstance: TemplateInstance | null = null;

                if (condMet && ssrCursor) {
                  const condBlockMeta = this.$block(condBlockIndex);
                  const condTplDom = condBlockMeta ? getTemplateDom(condBlockMeta) : null;

                  if (condBlockMeta && condTplDom) {
                    const condRootTag = this.$rootTag(condBlockMeta);
                    const wrapper = document.createElement('div');
                    if (condRootTag && ssrCursor.nodeType === 1
                        && (ssrCursor as Element).tagName.toLowerCase() === condRootTag) {
                      const claimed = ssrCursor;
                      ssrCursor = ssrCursor.nextSibling;
                      wrapper.appendChild(claimed);
                      condInstance = this.$hydrate(wrapper, condBlockMeta, condTplDom, itemScope);
                      condInstance.nodes = childNodesArray(wrapper);
                      for (const n of condInstance.nodes) {
                        ssrParent.insertBefore(n, ssrCursor);
                      }
                    } else {
                      condInstance = this.$createBlockInstance(condBlockIndex, itemScope);
                      if (condInstance) {
                        for (const n of condInstance.nodes) {
                          ssrParent.insertBefore(n, ssrCursor);
                        }
                      }
                    }
                  }
                }

                inst.conds.push({
                  condition, blockIndex: condBlockIndex,
                  anchor: condAnchor, scope: itemScope,
                  instance: condInstance,
                });
              }
            }

            repeatInsts.push({ key: String(j), value: itemValue, instance: inst });
          }
        }

        // Re-insert all nodes in correct order after anchor
        let cursor: Node = anchor;
        for (const ri of repeatInsts) {
          for (const n of ri.instance.nodes) {
            if (n.parentNode !== ssrParent || n.previousSibling !== cursor) {
              ssrParent.insertBefore(n, cursor.nextSibling);
            }
            cursor = n;
          }
        }

        instance.repeats.push({
          markerId: i, collection, itemVar, blockIndex,
          container: ssrParent as ParentNode & Node, start: anchor, end: null,
          scope, owner: instance, instances: repeatInsts, rootTag,
          attrMap, rootBindings, synced: true,
        });
      }
    }

    // Events + refs
    this.$finalize(ssrRoot, meta, (r, p) => this.$resolveSSR(r, tplDom, p));

    return instance;
  }

  // ── SSR helpers ───────────────────────────────────────────────

  /** Find existing SSR text node by mapping template text-node ordinal. */
  private $findSSRText(ssrParent: Node, tplParent: Node, beforeIndex: number): Text | null {
    // Use pre-computed ordinals from template cache
    const ordinals = getTplOrdinals(tplParent);
    let textOrd = 0;
    for (let k = 0; k < beforeIndex; k++) {
      const entry = ordinals.get(k);
      if (entry && entry[0] === 3) textOrd++;
    }

    // Find the text node at that ordinal in SSR DOM
    let count = 0;
    let child = ssrParent.firstChild;
    while (child) {
      if (child.nodeType === 3) {
        if (count === textOrd) return child as Text;
        count++;
      }
      child = child.nextSibling;
    }

    // Fallback: any text node with content
    child = ssrParent.firstChild;
    while (child) {
      if (child.nodeType === 3 && (child as Text).data && (child as Text).data.trim()) {
        return child as Text;
      }
      child = child.nextSibling;
    }
    return null;
  }

  /** Collect child elements matching a tag name. */
  private $collectByTag(parent: Node, tag: string): Node[] {
    const result: Node[] = [];
    let child = parent.firstChild;
    while (child) {
      if (child.nodeType === 1 && (child as Element).tagName.toLowerCase() === tag) {
        result.push(child);
      }
      child = child.nextSibling;
    }
    return result;
  }

  /** Collect non-empty text child nodes (for text-only condition blocks). */
  private $collectTextChildren(parent: Node): Node[] {
    const result: Node[] = [];
    let child = parent.firstChild;
    while (child) {
      if (child.nodeType === 3 && (child as Text).data && (child as Text).data.trim()) {
        result.push(child);
      }
      child = child.nextSibling;
    }
    return result;
  }

  /** Extract root tag name from block metadata. */
  private $rootTag(meta: TemplateBlockMeta): string | null {
    const h = meta.h;
    if (!h || h.charCodeAt(0) !== 60) return null;
    let end = 1;
    while (end < h.length) {
      const c = h.charCodeAt(end);
      if (c === 32 || c === 62 || c === 47) break;
      end++;
    }
    return h.slice(1, end).toLowerCase();
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

  /** Wire events + root events + refs (shared by $wire and $hydrate). */
  private $finalize(
    root: Node,
    meta: TemplateBlockMeta,
    resolver: (root: Node, path: TemplateNodePath) => Node | null,
  ): void {
    this.$wireEvents(root, meta, resolver);
    if ((meta as TemplateMeta).re) this.$wireRoot((meta as TemplateMeta).re!);
    this.$wireRefs(root);
  }

  /** Wire events using a resolver function (works for both client and SSR). */
  private $wireEvents(
    root: Node,
    meta: TemplateBlockMeta,
    resolver: (root: Node, path: TemplateNodePath) => Node | null,
  ): void {
    if (!meta.e) return;
    for (let i = 0; i < meta.e.length; i++) {
      const [eventName, handlerName, needsEvent, target] = meta.e[i];
      const el = resolver(root, target);
      if (!el || el.nodeType !== 1) continue;
      this.$addEvent(el as Element, eventName, handlerName, needsEvent);
    }
  }

  /** Wire root-level events on the host element (or shadow root when present). */
  private $wireRoot(re: [string, string, number][]): void {
    const target = this.shadowRoot ?? this;
    for (let i = 0; i < re.length; i++) {
      this.$addEvent(target, re[i][0], re[i][1], re[i][2]);
    }
  }

  /** Attach a single event listener. */
  private $addEvent(target: EventTarget, eventName: string, handlerName: string, needsEvent: number): void {
    const method = (this as Record<string, unknown>)[handlerName];
    if (typeof method !== 'function') return;
    target.addEventListener(eventName, (e: Event) => {
      if (needsEvent) {
        (method as (e: Event) => void).call(this, e);
      } else {
        (method as () => void).call(this);
      }
    });
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

  /** Create an AttrBinding from compiled metadata. */
  private $makeAttr(el: Element, entry: CompiledAttrMeta, scope?: ScopeFrame): AttrBinding {
    const name = entry[0];
    switch (entry[1]) {
      case 0: return { element: el, name, kind: 'attribute', path: entry[2] as string, scope };
      case 1: return { element: el, name, kind: 'complex', path: entry[2] as string, scope };
      case 2: return { element: el, name, kind: 'boolean', condition: entry[2] as CompiledCondition, scope };
      case 3: return { element: el, name, kind: 'template', parts: entry[2] as CompiledAttrPart[], scope };
      default: return { element: el, name, kind: 'attribute', path: '', scope };
    }
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

    // Merge wildcard into every concrete path
    const wc = index.get('*');
    if (wc) {
      index.delete('*');
      for (const [, e] of index) {
        e.texts.push(...wc.texts);
        e.attrs.push(...wc.attrs);
        e.conds.push(...wc.conds);
        e.repeats.push(...wc.repeats);
      }
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
      syncRepeat(this, this as unknown as Record<string, unknown>, this.constructor as Function, repeats[i]);
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
      case 'complex': {
        const v = this.$resolveValue(b.path!, b.scope);
        (el as unknown as Record<string, unknown>)[b.name] = v;
        break;
      }
      case 'boolean': {
        const show = b.condition![0](this.$resolver, b.scope);
        if (show) el.setAttribute(b.name, '');
        else el.removeAttribute(b.name);
        // Form control properties must be set via DOM property, not attribute
        if (b.name === 'checked' || b.name === 'selected' || b.name === 'disabled') {
          (el as unknown as Record<string, unknown>)[b.name] = show;
        }
        break;
      }
      case 'template': {
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
