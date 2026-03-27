// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { getTemplate } from './template.js';
import type { TemplateMeta } from './template.js';
import type { TemplateBlockMeta } from './template.js';
import {
  resolveNodePath,
  resolveNodePathFromNodes,
  resolveTemplateAlignedNodePathFromNodes,
  resolveElementPath,
  resolveSlotPath,
  isParentNode,
  matchesTemplateElement,
  collectElements,
  collectComments,
} from './element/paths.js';
import type {
  CompiledAttrMeta,
  CompiledAttrPart,
  CompiledConditionExpr,
  TemplateNodePath,
  TemplateSlotPath,
  CompiledTextRunMeta,
  CompiledAttrGroupMeta,
} from './template.js';
import { hydrationStart, hydrationEnd } from './lifecycle.js';
import { getObservableNames } from './decorators.js';
import {
  deriveConditionSeed,
  evaluateCondition,
} from './element/conditions.js';
import { readHydrationEventCounts } from './element/events.js';
import { seedObservablePath } from './element/seed.js';
import {
  setupRepeat,
  syncRepeat,
  readBlockMarkerName,
  resolveRepeatValue,
  repeatBlockMetadata,
} from './element/repeat.js';
import { getModuleStylesheet } from './element/styles.js';
import type {
  AttrBinding,
  CondBinding,
  RepeatBinding,
  RepeatHost,
  RepeatItemInstance,
  ResolvedSlot,
  ScopeFrame,
  TemplateInstance,
  TextBinding,
} from './element/types.js';
import { toCamelCase } from './element/types.js';

/**
 * Base class for WebUI web components.
 *
 * ## Lifecycle
 *
 * 1. **SSR path**: The server renders the component with a Declarative Shadow
 *    Root (`<template shadowrootmode>`). On upgrade, `connectedCallback` finds
 *    the existing shadow root, walks it once to connect bindings, then sets
 *    `$ready = true`.
 *
 * 2. **Client-created path**: When a component is created dynamically (e.g.
 *    inside a `<for>` loop), `connectedCallback` creates a shadow root from
 *    marker-free compiled HTML, resolves precompiled locator metadata once,
 *    and then sets `$ready = true`.
 *
 * In both cases, after the single setup pass, all reactive updates are
 * **O(affected) direct node patches** — a per-path index maps each
 * `@observable` property to only the bindings that reference it.  No
 * scanning, no regex, no selector queries.  The `$update(path)` method
 * looks up affected bindings and patches them directly.
 *
 * ## Binding arrays
 *
 * - `$t` — Text bindings: direct `Text` nodes wired from SSR markers or
 *   client text-run locators
 * - `$c` — Conditionals: `{ condition, blockIndex, anchor, instance }` — toggle compiled blocks
 * - `$r` — Repeats: `{ collection, itemVar, blockIndex, container }` — reconcile child instances
 *
 * ## Custom element upgrade order
 *
 * Per the HTML spec, when a custom element is inserted into the DOM:
 * 1. Constructor runs
 * 2. `attributeChangedCallback` fires for pre-existing attributes
 * 3. `connectedCallback` fires
 *
 * The `$ready` guard prevents `$update()` from running during step 2 (before
 * bindings are connected). After `connectedCallback` finishes setup, it calls
 * `$update()` to flush any property values that were set during step 2.
 */
/**
 * Parsed template cache — avoids re-parsing the same `meta.h` HTML string
 * for every client-created instance.  Keyed on the metadata object itself
 * (same object identity for same component tag).  `cloneNode(true)` is
 * significantly faster than `innerHTML` parsing for repeated instantiation
 * (e.g. 200 items in a `@for` loop).
 */
const templateCache = new WeakMap<TemplateBlockMeta, DocumentFragment>();

export class WebUIElement extends HTMLElement {
  /** Root compiled template instance. */
  private $root: TemplateInstance | null = null;
  /** Root compiled metadata for nested block lookup. */
  private $meta?: TemplateMeta;
  /** Set to `true` after `connectedCallback` finishes hydration.
   *  Guards `$update()` from running before bindings are connected. */
  private $ready = false;
  /** Set after `$hydrate()` finishes wiring the component's SSR/client DOM. */
  private $hydrated = false;
  /**
   * Per-path binding index for targeted updates.
   *
   * Maps root property names to the subset of bindings that reference them.
   * When `@observable count` changes, `$update('count')` only walks
   * bindings in `$pathIndex.get('count')` instead of all bindings.
   */
  private $pathIndex?: Map<string, {
    texts: TextBinding[];
    attrs: AttrBinding[];
    conds: CondBinding[];
    repeats: RepeatBinding[];
  }>;

  /** Register this class as a custom element with the given tag name. */
  static define(tagName: string): void {
    customElements.define(tagName, this);
  }

  /**
   * Called by the browser when the element is inserted into the DOM.
   *
   * Handles two paths:
   * - **SSR**: Shadow root already exists (Declarative Shadow DOM) — hydrate it.
   * - **Client-created**: No shadow root — create one from compiled metadata.
   *
   * After setup, calls `$update()` to flush any property values that were
   * set during the custom element upgrade (via `attributeChangedCallback`).
   */
  connectedCallback(): void {
    const tag = this.tagName.toLowerCase();
    hydrationStart(tag);

    if (this.$hydrated && this.$root) {
      this.$ready = true;
      this.$update();
      hydrationEnd(tag);
      return;
    }

    const meta = getTemplate(tag);
    this.$meta = meta;
    const isSSR = !!this.shadowRoot;

    if (!isSSR && meta) {
      this.$createFromMeta(meta);
    }

    if (isSSR && this.shadowRoot) {
      this.$hydrate(meta, isSSR);
    }

    this.$buildPathIndex();
    this.$ready = true;

    if (!isSSR) {
      this.$update();
    }

    hydrationEnd(tag);
  }

  /** Lifecycle hook for subclasses that attach global listeners. */
  disconnectedCallback(): void {}

  /**
   * Dispatch a bubbling, composed custom event from this element.
   * Events cross shadow DOM boundaries (`composed: true`).
   */
  $emit(name: string, detail?: unknown): boolean {
    return this.dispatchEvent(
      new CustomEvent(name, {
        bubbles: true,
        composed: true,
        cancelable: true,
        detail,
      }),
    );
  }

  /**
   * Auto setInitialState — populates @observable properties from router state.
   */
  setInitialState(state: Record<string, unknown>): void {
    const names = getObservableNames(this.constructor as Function);
    for (const key of Object.keys(state)) {
      if (names.has(key)) {
        (this as Record<string, unknown>)[key] = state[key];
      }
    }
  }


  private $singleDynamicAttrPart(parts: CompiledAttrPart[]): {
    path: string;
    prefix: string;
    suffix: string;
  } | null {
    let path = '';
    let prefix = '';
    let suffix = '';
    let seenDynamic = false;

    for (const part of parts) {
      if (typeof part === 'string') {
        if (seenDynamic) {
          suffix += part;
        } else {
          prefix += part;
        }
        continue;
      }

      if (seenDynamic) {
        return null;
      }

      path = part[0];
      seenDynamic = true;
    }

    return seenDynamic ? { path, prefix, suffix } : null;
  }

  private $stripAffixes(raw: string, prefix: string, suffix: string): string | undefined {
    if (!raw.startsWith(prefix) || !raw.endsWith(suffix)) {
      return undefined;
    }

    const end = suffix.length > 0 ? raw.length - suffix.length : raw.length;
    return raw.slice(prefix.length, end);
  }

  /**
   * Reactive update. Called by @observable/@attr setters.
   *
   * When `path` is provided (e.g. `'count'`), only updates bindings that
   * reference that property — O(affected) instead of O(all).
   * Falls back to full update when no path is given or when the path
   * has no indexed bindings.
   *
   * Bindings that reference computed/volatile properties (paths not in
   * the `@observable` set) are pre-merged into each path's entry at
   * index build time, so targeted updates require zero allocations.
   */
  $update(path?: string): void {
    if (!this.$ready || !this.$root) return;

    if (path && this.$pathIndex) {
      const entry = this.$pathIndex.get(path);
      if (entry) {
        this.$updateBindings(entry.texts, entry.attrs, entry.conds, entry.repeats);
        return;
      }
    }

    this.$updateInstance(this.$root);
  }

  /**
   * Build the per-path binding index from the root template instance.
   *
   * Maps each root property name to the subset of bindings that
   * reference it, so `$update(path)` can skip unaffected bindings.
   */
  private $buildPathIndex(): void {
    if (!this.$root) return;

    const observableNames = getObservableNames(this.constructor as Function);
    const index = new Map<string, {
      texts: TextBinding[];
      attrs: AttrBinding[];
      conds: CondBinding[];
      repeats: RepeatBinding[];
    }>();

    const ensure = (path: string) => {
      let entry = index.get(path);
      if (!entry) {
        entry = { texts: [], attrs: [], conds: [], repeats: [] };
        index.set(path, entry);
      }
      return entry;
    };

    // Root property name, or '*' for computed/volatile bindings
    const keyFor = (path: string) => {
      const root = path.split('.')[0];
      return observableNames.has(root) ? root : '*';
    };

    for (const binding of this.$root.texts) {
      if (binding.path) {
        ensure(keyFor(binding.path)).texts.push(binding);
      }
      if (binding.parts) {
        for (const part of binding.parts) {
          if (typeof part !== 'string') {
            ensure(keyFor(part[0])).texts.push(binding);
          }
        }
      }
    }

    for (const binding of this.$root.attrs) {
      if (binding.path) {
        ensure(keyFor(binding.path)).attrs.push(binding);
      }
      if (binding.condition) {
        this.$indexConditionPaths(binding.condition, (p) => ensure(keyFor(p)).attrs.push(binding));
      }
      if (binding.parts) {
        for (const part of binding.parts) {
          if (typeof part !== 'string') {
            ensure(keyFor(part[0])).attrs.push(binding);
          }
        }
      }
    }

    for (const binding of this.$root.conds) {
      this.$indexConditionPaths(binding.condition, (p) => ensure(keyFor(p)).conds.push(binding));
    }

    for (const binding of this.$root.repeats) {
      ensure(keyFor(binding.collection)).repeats.push(binding);
    }

    // Pre-merge wildcard (volatile/computed) bindings into every path entry
    // so that $update(path) is a single map lookup with zero allocations.
    const wild = index.get('*');
    if (wild) {
      for (const [key, entry] of index) {
        if (key === '*') continue;
        if (wild.texts.length > 0) entry.texts.push(...wild.texts);
        if (wild.attrs.length > 0) entry.attrs.push(...wild.attrs);
        if (wild.conds.length > 0) entry.conds.push(...wild.conds);
        if (wild.repeats.length > 0) entry.repeats.push(...wild.repeats);
      }
      index.delete('*');
    }

    this.$pathIndex = index;
  }

  /**
   * Walk a condition AST and call `register` for each root property it references.
   */
  private $indexConditionPaths(
    condition: CompiledConditionExpr,
    register: (rootPath: string) => void,
  ): void {
    const stack: CompiledConditionExpr[] = [condition];
    const seen = new Set<string>();

    while (stack.length > 0) {
      const current = stack.pop()!;
      switch (current[0]) {
        case 0: { // identifier
          const root = current[1].split('.')[0];
          if (!seen.has(root)) {
            seen.add(root);
            register(root);
          }
          break;
        }
        case 1: { // predicate
          const leftRoot = current[1].split('.')[0];
          if (!seen.has(leftRoot)) {
            seen.add(leftRoot);
            register(leftRoot);
          }
          // right side might be a path too (not a literal)
          const right = current[3];
          if (!right.startsWith('"') && !right.startsWith("'") && right !== 'true' && right !== 'false' && !/^-?\d/.test(right)) {
            const rightRoot = right.split('.')[0];
            if (!seen.has(rightRoot)) {
              seen.add(rightRoot);
              register(rightRoot);
            }
          }
          break;
        }
        case 2: // not
          stack.push(current[1]);
          break;
        case 3: // compound
          stack.push(current[3]);
          stack.push(current[1]);
          break;
      }
    }
  }

  // ── Setup ────────────────────────────────────────────────────────

  /** Create shadow root from metadata (client-only components). */
  private $createFromMeta(meta: TemplateMeta): void {
    const sr = this.attachShadow({ mode: 'open' });

    if (meta.sa) {
      const stylesheet = getModuleStylesheet(meta.sa);
      if (!sr.adoptedStyleSheets.includes(stylesheet)) {
        sr.adoptedStyleSheets = [...sr.adoptedStyleSheets, stylesheet];
      }
    }

    this.$root = this.$buildClientInstance(meta);
    this.$bindRefs(this.$root.nodes);
    sr.appendChild(this.$fragmentFromNodes(this.$root.nodes));

    if (meta.re) {
      for (const [event, handler, needsEvent] of meta.re) {
        this.$wire(this, event, handler, !!needsEvent);
      }
    }

    this.$hydrated = true;
  }

  /** One-time hydration: connect all bindings from metadata + SSR DOM. */
  private $hydrate(meta?: TemplateMeta, seedSSR = false): void {
    const sr = this.shadowRoot!;

    // Refs: bind w-ref elements directly to component properties.
    // w-ref="name" or w-ref={name} → this[name] = element
    this.$bindRefs(Array.from(sr.childNodes));

    // When seeding, compute observable names once and pass them through
    // so that $walkMarkers and $connectAttrBindings can seed inline.
    const observableNames = seedSSR
      ? getObservableNames(this.constructor as Function)
      : undefined;
    const seededPaths = observableNames ? new Set<string>() : undefined;

    this.$root = this.$createTemplateInstance(
      Array.from(sr.childNodes),
      meta,
      undefined,
      observableNames,
      seededPaths,
    );

    // Root events from metadata
    if (meta?.re) {
      for (const [event, handler, needsEvent] of meta.re) {
        this.$wire(this, event, handler, !!needsEvent);
      }
    }

    this.$hydrated = true;

    // Clean SSR markers
    this.$clean(sr);
  }

  private $buildClientInstance(
    meta?: TemplateBlockMeta,
    scope?: ScopeFrame,
  ): TemplateInstance {
    if (!meta) {
      return {
        scope,
        nodes: [],
        texts: [],
        attrs: [],
        conds: [],
        repeats: [],
      };
    }

    let cached = templateCache.get(meta);
    if (!cached) {
      const template = document.createElement('template');
      template.innerHTML = meta.h;
      cached = template.content;
      templateCache.set(meta, cached);
    }
    const fragment = cached.cloneNode(true) as DocumentFragment;
    const nodes = Array.from(fragment.childNodes);

    return this.$createClientTemplateInstance(fragment, nodes, meta, scope);
  }

  private $bindRefs(nodes: Node[]): void {
    for (const el of collectElements(nodes)) {
      if (!el.hasAttribute('w-ref')) {
        continue;
      }

      let name = el.getAttribute('w-ref')!;
      name = name.replace(/^\{|\}$/g, '');
      (this as unknown as Record<string, HTMLElement>)[name] = el as HTMLElement;
    }
  }

  private $createTemplateInstance(
    nodes: Node[],
    meta?: TemplateBlockMeta,
    scope?: ScopeFrame,
    observableNames?: Set<string>,
    seededPaths?: Set<string>,
  ): TemplateInstance {
    const instance: TemplateInstance = {
      scope,
      nodes,
      texts: [],
      attrs: [],
      conds: [],
      repeats: [],
    };

    if (!meta) {
      return instance;
    }

    this.$connectAttrBindings(nodes, meta, scope, instance, observableNames, seededPaths);
    this.$walkMarkers(nodes, meta, scope, instance, observableNames, seededPaths);

    if (meta.r) {
      for (let index = 0; index < meta.r.length; index += 1) {
        const [collection, itemVar, blockIndex] = meta.r[index];
        setupRepeat(this as unknown as RepeatHost, this as unknown as Record<string, unknown>, this.constructor as Function, nodes, instance, index, collection, itemVar, blockIndex, scope, meta, meta.rl?.[index]);
      }
    }

    this.$wireTemplateEvents(nodes, meta.e);
    return instance;
  }

  private $replaceInstanceNode(
    instance: TemplateInstance,
    current: Node,
    replacement: Node,
  ): void {
    instance.nodes = instance.nodes.map((node) => (node === current ? replacement : node));
  }

  private $removeInstanceNode(instance: TemplateInstance, node: Node): void {
    instance.nodes = instance.nodes.filter((entry) => entry !== node);
  }

  private $createClientTemplateInstance(
    root: ParentNode & Node,
    nodes: Node[],
    meta: TemplateBlockMeta,
    scope?: ScopeFrame,
  ): TemplateInstance {
    const instance: TemplateInstance = {
      scope,
      nodes,
      texts: [],
      attrs: [],
      conds: [],
      repeats: [],
    };

    const attrTargets = (meta.ag ?? [])
      .map((group) => this.$resolveAttrGroup(root, group))
      .filter((target): target is { element: Element; start: number; count: number } => target !== null);
    const eventTargets = (meta.el ?? [])
      .map((path) => resolveElementPath(root, path))
      .filter((element): element is Element => element !== null);

    const insertions: Array<
      | { kind: 'text'; slot: ResolvedSlot; parts: CompiledAttrPart[] }
      | { kind: 'cond'; slot: ResolvedSlot; markerId: number; entry: [CompiledConditionExpr, number] }
      | { kind: 'repeat'; slot: ResolvedSlot; markerId: number; entry: [string, string, number] }
    > = [];

    for (const [index, [slotPath, parts]] of (meta.tx ?? []).entries()) {
      const slot = resolveSlotPath(root, slotPath);
      if (!slot) {
        continue;
      }

      insertions.push({
        kind: 'text',
        slot,
        parts,
      });
    }

    for (const [index, entry] of (meta.c ?? []).entries()) {
      const slotPath = meta.cl?.[index];
      if (!slotPath) {
        continue;
      }

      const slot = resolveSlotPath(root, slotPath);
      if (!slot) {
        continue;
      }

      insertions.push({
        kind: 'cond',
        slot,
        markerId: index,
        entry,
      });
    }

    for (const [index, entry] of (meta.r ?? []).entries()) {
      const slotPath = meta.rl?.[index];
      if (!slotPath) {
        continue;
      }

      const slot = resolveSlotPath(root, slotPath);
      if (!slot) {
        continue;
      }

      insertions.push({
        kind: 'repeat',
        slot,
        markerId: index,
        entry,
      });
    }

    insertions.sort((left, right) => left.slot.order - right.slot.order);

    for (const insertion of insertions) {
      if (insertion.kind === 'text') {
        const node = document.createTextNode('');
        insertion.slot.parent.insertBefore(node, insertion.slot.nextSibling);
        instance.texts.push({
          node,
          parts: insertion.parts,
          scope,
        });
        continue;
      }

      if (insertion.kind === 'cond') {
        const anchor = document.createComment(`c:${insertion.markerId}`);
        insertion.slot.parent.insertBefore(anchor, insertion.slot.nextSibling);
        instance.conds.push({
          condition: insertion.entry[0],
          blockIndex: insertion.entry[1],
          anchor,
          scope,
          instance: null,
        });
        continue;
      }

      const anchor = document.createComment(`r:${insertion.markerId}`);
      insertion.slot.parent.insertBefore(anchor, insertion.slot.nextSibling);
      const [collection, itemVar, blockIndex] = insertion.entry;
      const { rootTag, attrMap, rootBindings } = repeatBlockMetadata(this as unknown as RepeatHost, blockIndex, itemVar);
      instance.repeats.push({
        markerId: insertion.markerId,
        collection,
        itemVar,
        blockIndex,
        container: insertion.slot.parent,
        start: anchor,
        end: null,
        scope,
        owner: instance,
        instances: [],
        rootTag,
        attrMap,
        rootBindings,
      });
    }

    for (const target of attrTargets) {
      for (let index = target.start; index < target.start + target.count; index += 1) {
        const entry = meta.a?.[index];
        if (entry) {
          this.$connectAttrBinding(target.element, entry, scope, instance);
        }
      }
    }

    for (let index = 0; index < eventTargets.length; index += 1) {
      const entry = meta.e?.[index];
      if (!entry) {
        continue;
      }

      const [event, handler, needsEvent] = entry;
      this.$wire(eventTargets[index], event, handler, !!needsEvent);
    }

    instance.nodes = Array.from(root.childNodes);
    return instance;
  }

  private $createBlockInstance(blockIndex: number, scope?: ScopeFrame): TemplateInstance | null {
    const block = this.$block(blockIndex);
    if (!block) {
      return null;
    }

    return this.$buildClientInstance(block, scope);
  }

  private $hydrateExistingBlockInstance(
    blockIndex: number,
    nodes: Node[],
    scope?: ScopeFrame,
  ): TemplateInstance | null {
    const block = this.$block(blockIndex);
    if (!block) {
      return null;
    }

    return this.$createTemplateInstance(nodes, block, scope);
  }

  private $block(blockIndex: number): TemplateBlockMeta | undefined {
    return this.$meta?.b?.[blockIndex];
  }

  private $updateInstance(instance: TemplateInstance): void {
    this.$updateBindings(instance.texts, instance.attrs, instance.conds, instance.repeats);
  }

  private $updateBindings(
    texts: readonly TextBinding[],
    attrs: readonly AttrBinding[],
    conds: readonly CondBinding[],
    repeats: readonly RepeatBinding[],
  ): void {
    for (const binding of texts) {
      const str = binding.parts
        ? this.$resolveAttrParts(binding.parts, binding.scope)
        : (() => {
            const val = binding.path ? this.$resolveValue(binding.path, binding.scope) : undefined;
            return val != null ? String(val) : '';
          })();
      if (binding.node.textContent !== str) {
        binding.node.textContent = str;
      }
    }

    for (const binding of attrs) {
      if (binding.kind === 'complex') {
        const val = binding.path ? this.$resolveValue(binding.path, binding.scope) : undefined;
        const target = binding.element as unknown as Record<string, unknown>;
        if (target[binding.name] !== val) {
          target[binding.name] = val;
        }
        continue;
      }

      if (binding.kind === 'boolean') {
        const truthy = binding.condition
          ? evaluateCondition(binding.condition, (path, currentScope) => this.$resolveValue(path, currentScope), binding.scope)
          : false;
        if (truthy) {
          if (!binding.element.hasAttribute(binding.name)) {
            binding.element.setAttribute(binding.name, '');
          }
        } else if (binding.element.hasAttribute(binding.name)) {
          binding.element.removeAttribute(binding.name);
        }
        continue;
      }

      const str = binding.kind === 'template'
        ? this.$resolveAttrParts(binding.parts ?? [], binding.scope)
        : (() => {
            const val = binding.path ? this.$resolveValue(binding.path, binding.scope) : undefined;
            return val != null ? String(val) : '';
          })();
      if (binding.element.getAttribute(binding.name) !== str) {
        binding.element.setAttribute(binding.name, str);
      }
    }

    for (const binding of conds) {
      this.$toggleCond(binding);
    }

    for (const binding of repeats) {
      syncRepeat(this as unknown as RepeatHost, this as unknown as Record<string, unknown>, this.constructor as Function, binding);
    }
  }

  private $destroyInstance(instance: TemplateInstance): void {
    for (const binding of instance.conds) {
      if (binding.instance) {
        this.$destroyInstance(binding.instance);
        binding.instance = null;
      }
    }

    for (const binding of instance.repeats) {
      for (const item of binding.instances) {
        this.$destroyInstance(item.instance);
      }
      binding.instances = [];
    }
  }

  private $removeInstance(instance: TemplateInstance): void {
    this.$destroyInstance(instance);
    for (const node of instance.nodes) {
      node.parentNode?.removeChild(node);
    }
  }

  private $fragmentFromNodes(nodes: Node[]): DocumentFragment {
    const fragment = document.createDocumentFragment();
    for (const node of nodes) {
      fragment.appendChild(node);
    }
    return fragment;
  }

  private $insertInstanceAfter(
    cursor: Node | null,
    container: ParentNode & Node,
    instance: TemplateInstance,
  ): Node | null {
    const first = instance.nodes[0] ?? null;
    const expected = cursor?.parentNode === container
      ? cursor.nextSibling
      : container.firstChild;
    if (first && first.parentNode === container && first === expected) {
      return instance.nodes[instance.nodes.length - 1] ?? cursor;
    }

    const fragment = this.$fragmentFromNodes(instance.nodes);
    const last = fragment.lastChild;
    if (cursor?.parentNode === container) {
      cursor.parentNode.insertBefore(fragment, cursor.nextSibling);
    } else {
      container.appendChild(fragment);
    }
    return last ?? cursor;
  }

  // ── SSR hydration walk (single pass) ─────────────────────────────

  private $connectAttrBindings(
    nodes: Node[],
    meta: TemplateBlockMeta,
    scope: ScopeFrame | undefined,
    instance: TemplateInstance,
    observableNames?: Set<string>,
    seededPaths?: Set<string>,
  ): void {
    if (!meta.a) {
      return;
    }

    if (meta.ag?.length) {
      let resolvedAllGroups = true;
      for (const group of meta.ag) {
        const target = this.$resolveHydrationAttrGroup(nodes, meta, group);
        if (!target) {
          resolvedAllGroups = false;
          break;
        }

        for (let idx = target.start; idx < target.start + target.count; idx += 1) {
          const entry = meta.a[idx];
          if (entry) {
            this.$connectAttrBinding(target.element, entry, scope, instance, observableNames, seededPaths);
          }
        }
      }

      if (resolvedAllGroups) {
        return;
      }
    }

    const elements = this.$collectHydrationAttrElements(nodes);
    for (const el of elements) {
      const bindingRange = this.$readAttrBindingRange(el);
      if (!bindingRange) {
        continue;
      }

      const [start, count] = bindingRange;
      for (let idx = start; idx < start + count; idx += 1) {
        const entry = meta.a[idx];
        if (entry) {
          this.$connectAttrBinding(el, entry, scope, instance, observableNames, seededPaths);
        }
      }
    }
  }

  private $readAttrBindingRange(el: Element): [number, number] | null {
    for (const attr of Array.from(el.attributes)) {
      if (attr.name.startsWith('data-w-b-')) {
        const start = parseInt(attr.name.slice('data-w-b-'.length), 10);
        if (!Number.isNaN(start)) {
          return [start, 1];
        }
      }

      if (attr.name.startsWith('data-w-c-')) {
        const parts = attr.name.slice('data-w-c-'.length).split('-');
        if (parts.length === 2) {
          const start = parseInt(parts[0], 10);
          const count = parseInt(parts[1], 10);
          if (!Number.isNaN(start) && !Number.isNaN(count)) {
            return [start, count];
          }
        }
      }
    }

    return null;
  }

  private $connectAttrBinding(
    el: Element,
    entry: CompiledAttrMeta,
    scope: ScopeFrame | undefined,
    instance: TemplateInstance,
    observableNames?: Set<string>,
    seededPaths?: Set<string>,
  ): void {
    const [name, kind, payload] = entry;
    if (kind === 0) {
      instance.attrs.push({
        element: el,
        name,
        kind: 'attribute',
        path: payload,
        scope,
      });
      // Seed: simple attribute binding — read the DOM attribute value
      if (observableNames && payload) {
        const raw = el.getAttribute(name);
        if (raw != null) {
          seedObservablePath(this as unknown as Record<string, unknown>, payload, raw, observableNames, seededPaths);
        }
      }
      return;
    }

    if (kind === 1) {
      const propName = toCamelCase(name.slice(1));
      instance.attrs.push({
        element: el,
        name: propName,
        kind: 'complex',
        path: payload,
        scope,
      });
      // Seed: complex property binding — read the child component's property
      if (observableNames && payload) {
        const target = el as unknown as Record<string, unknown>;
        let value = target[propName];
        if (
          (value === undefined || (Array.isArray(value) && value.length === 0))
          && el instanceof WebUIElement
        ) {
          // Trigger child hydration/seeding first so the property is populated
          el.$hydrate(el.$meta, true);
          value = target[propName];
        }
        if (value !== undefined) {
          seedObservablePath(this as unknown as Record<string, unknown>, payload, value, observableNames, seededPaths);
        }
      }
      return;
    }

    if (kind === 2) {
      instance.attrs.push({
        element: el,
        name,
        kind: 'boolean',
        condition: payload,
        scope,
      });
      // Seed: boolean attribute — infer observable value from condition + presence
      if (observableNames) {
        const seed = deriveConditionSeed(payload, el.hasAttribute(name));
        if (seed) {
          if (seed.kind === 'empty-collection') {
            if (!seededPaths?.has(seed.path)) {
              seedObservablePath(this as unknown as Record<string, unknown>, seed.path, [], observableNames, seededPaths);
            }
          } else if (!seededPaths?.has(seed.path)) {
            seedObservablePath(this as unknown as Record<string, unknown>, seed.path, seed.value, observableNames, seededPaths);
          }
        }
      }
      return;
    }

    instance.attrs.push({
      element: el,
      name,
      kind: 'template',
      parts: payload,
      scope,
    });
    // Seed: template interpolation — extract the dynamic part from the rendered attribute
    if (observableNames) {
      const dynamic = this.$singleDynamicAttrPart(payload);
      if (dynamic) {
        const raw = el.getAttribute(name);
        if (raw != null) {
          const value = this.$stripAffixes(raw, dynamic.prefix, dynamic.suffix);
          if (value !== undefined) {
            seedObservablePath(this as unknown as Record<string, unknown>, dynamic.path, value, observableNames, seededPaths);
          }
        }
      }
    }
  }

  private $collectHydrationAttrElements(nodes: readonly Node[]): Element[] {
    const elements: Element[] = [];
    const stack: Array<{ nodes: Node[]; index: number }> = [{
      nodes: [...nodes],
      index: 0,
    }];

    while (stack.length > 0) {
      const frame = stack[stack.length - 1];
      if (frame.index >= frame.nodes.length) {
        stack.pop();
        continue;
      }

      const node = frame.nodes[frame.index];
      frame.index += 1;

      if (node instanceof Comment) {
        const name = readBlockMarkerName(node.data, 'w-b:start:');
        if (name?.startsWith('if-') || name?.startsWith('for-')) {
          this.$skipHydrationBlock(frame, name);
        }
        continue;
      }

      if (!(node instanceof Element)) {
        continue;
      }

      elements.push(node);
      if (node.childNodes.length > 0) {
        stack.push({
          nodes: Array.from(node.childNodes),
          index: 0,
        });
      }
    }

    return elements;
  }

  private $skipHydrationBlock(
    frame: { nodes: Node[]; index: number },
    name: string,
  ): void {
    let depth = 0;
    while (frame.index < frame.nodes.length) {
      const node = frame.nodes[frame.index];
      frame.index += 1;

      if (!(node instanceof Comment)) {
        continue;
      }

      const nestedStart = readBlockMarkerName(node.data, 'w-b:start:');
      if (nestedStart === name) {
        depth += 1;
        continue;
      }

      const endName = readBlockMarkerName(node.data, 'w-b:end:');
      if (endName !== name) {
        continue;
      }

      if (depth === 0) {
        return;
      }

      depth -= 1;
    }
  }

  private $resolveAttrGroup(
    root: ParentNode & Node,
    group: CompiledAttrGroupMeta,
  ): { element: Element; start: number; count: number } | null {
    const [path, start, count] = group;
    const element = resolveElementPath(root, path);
    if (!element) {
      return null;
    }

    return { element, start, count };
  }

  private $resolveHydrationAttrGroup(
    nodes: Node[],
    meta: TemplateBlockMeta,
    group: CompiledAttrGroupMeta,
  ): { element: Element; start: number; count: number } | null {
    const [path, start, count] = group;
    const template = document.createElement('template');
    template.innerHTML = meta.h;
    const reference = resolveNodePath(template.content, path);
    const direct = resolveNodePathFromNodes(nodes, path);
    const aligned = resolveTemplateAlignedNodePathFromNodes(nodes, template.content, path);

    let element: Element | null = null;
    if (reference instanceof Element) {
      if (direct instanceof Element && matchesTemplateElement(direct, reference)) {
        element = direct;
      } else if (aligned instanceof Element && matchesTemplateElement(aligned, reference)) {
        element = aligned;
      } else {
        element = this.$collectHydrationAttrElements(nodes)
          .find((candidate) => matchesTemplateElement(candidate, reference)) ?? null;
      }
    } else if (direct instanceof Element) {
      element = direct;
    } else if (aligned instanceof Element) {
      element = aligned;
    }

    return element ? { element, start, count } : null;
  }

  private $resolveSlotParentFromNodes(
    nodes: Node[],
    meta: TemplateBlockMeta,
    slotPath: TemplateSlotPath,
  ): (ParentNode & Node) | null {
    const [parentPath] = slotPath;
    const template = document.createElement('template');
    template.innerHTML = meta.h;
    const reference = resolveNodePath(template.content, parentPath);
    const parentNode = resolveNodePathFromNodes(nodes, parentPath);
    const alignedNode = resolveTemplateAlignedNodePathFromNodes(
      nodes,
      template.content,
      parentPath,
    );

    if (!(reference instanceof Element)) {
      if (alignedNode && isParentNode(alignedNode)) {
        return alignedNode;
      }
      return parentNode && isParentNode(parentNode) ? parentNode : null;
    }

    if (parentNode instanceof Element && matchesTemplateElement(parentNode, reference)) {
      return parentNode;
    }

    if (alignedNode instanceof Element && matchesTemplateElement(alignedNode, reference)) {
      return alignedNode;
    }

    const candidates = collectElements(nodes)
      .filter((element) => matchesTemplateElement(element, reference));
    return candidates[0]
      ?? (alignedNode && isParentNode(alignedNode) ? alignedNode : null)
      ?? (parentNode && isParentNode(parentNode) ? parentNode : null);
  }

  private $resolveAttrParts(parts: CompiledAttrPart[], scope?: ScopeFrame): string {
    let result = '';
    for (const part of parts) {
      if (typeof part === 'string') {
        result += part;
        continue;
      }

      const val = this.$resolveValue(part[0], scope);
      result += val != null ? String(val) : '';
    }

    return result;
  }

  /**
   * ONE TreeWalker pass over SSR hydration comments in existing DOM.
   *
   * Client-created DOM never relies on compiled comment markers. This walk is
   * only for server-rendered `w-b:*` / `w-r:*` markers that reconstruct the
   * already-rendered DOM into direct binding references.
   */
  private $walkMarkers(
    nodes: Node[],
    meta: TemplateBlockMeta,
    scope: ScopeFrame | undefined,
    instance: TemplateInstance,
    observableNames?: Set<string>,
    seededPaths?: Set<string>,
  ): void {
    const ssrIfStarts = new Map<string, Comment>();
    // SSR `if-N` identifiers are emitted from the full render graph, not the
    // local block's `meta.c` array. Re-map them to local encounter order so
    // hydrated block instances can reconnect their conditional bindings.
    const ssrIfIndices = new Map<string, number>();
    let nextSsrIfIndex = 0;
    let repeatDepth = 0; // track nesting inside w-r: (for-loop) scopes
    for (const c of collectComments(nodes)) {
      const d = c.data;

      // Track repeat scope depth — SSR bindings inside for-loops belong
      // to the loop item, not the component, so we skip them.
      if (d.startsWith('w-r:start:')) {
        repeatDepth++;
        continue;
      }
      if (d.startsWith('w-r:end:')) {
        repeatDepth--;
        continue;
      }

      // SSR marker: w-b:start:N:name (text binding from SSR)
      // Skip if inside a repeat scope — those bindings belong to loop items.
      if (d.startsWith('w-b:start:')) {
        if (repeatDepth > 0) continue;
        const parts = d.slice('w-b:start:'.length).split(':');
        if (parts.length >= 2) {
          const name = parts.slice(1).join(':');

          if (name.startsWith('if-')) {
            ssrIfStarts.set(name, c);
            if (!ssrIfIndices.has(name)) {
              ssrIfIndices.set(name, nextSsrIfIndex);
              nextSsrIfIndex += 1;
            }
          } else if (!name.startsWith('for-')) {
            const text = c.nextSibling;
            if (text instanceof Text) {
              instance.texts.push({ path: name, node: text, scope });
              // Seed inline: we already have the path and the SSR text content
              if (observableNames) {
                seedObservablePath(
                  this as unknown as Record<string, unknown>,
                  name,
                  text.textContent ?? '',
                  observableNames,
                  seededPaths,
                );
              }
            }
          }
        }
        continue;
      }

      // SSR marker: w-b:end:N:if-M (conditional end)
      // Skip if inside a repeat scope.
      if (d.startsWith('w-b:end:')) {
        if (repeatDepth > 0) continue;
        const parts = d.slice('w-b:end:'.length).split(':');
        if (parts.length >= 2) {
          const name = parts.slice(1).join(':');
          if (name.startsWith('if-')) {
            const start = ssrIfStarts.get(name);
            const ifIdx = ssrIfIndices.get(name);
            if (start && ifIdx !== undefined) {
              const entry = meta.c?.[ifIdx];
              if (entry) {
                const existingNodes: Node[] = [];
                for (let n = start.nextSibling; n && n !== c; n = n.nextSibling) {
                  if (n instanceof Text && !n.data.trim()) continue;
                  existingNodes.push(n);
                }
                const anchor = document.createComment(`c:${ifIdx}`);
                start.replaceWith(anchor);
                this.$replaceInstanceNode(instance, start, anchor);
                c.remove();
                this.$removeInstanceNode(instance, c);
                const shown = existingNodes.length > 0;
                instance.conds.push({
                  condition: entry[0],
                  blockIndex: entry[1],
                  anchor,
                  scope,
                  instance: shown
                    ? this.$hydrateExistingBlockInstance(entry[1], existingNodes, scope)
                    : null,
                });
                // Seed inline: infer observable value from condition visibility
                if (observableNames) {
                  const seed = deriveConditionSeed(entry[0], shown);
                  if (seed) {
                    if (seed.kind === 'empty-collection') {
                      if (!seededPaths?.has(seed.path)) {
                        seedObservablePath(this as unknown as Record<string, unknown>, seed.path, [], observableNames, seededPaths);
                      }
                    } else if (!seededPaths?.has(seed.path)) {
                      seedObservablePath(this as unknown as Record<string, unknown>, seed.path, seed.value, observableNames, seededPaths);
                    }
                  }
                }
              }
              ssrIfStarts.delete(name);
              ssrIfIndices.delete(name);
            }
          }
        }
        continue;
      }
    }
  }

  private $wireTemplateEvents(
    nodes: Node[],
    events?: [string, string, number][],
  ): void {
    if (!events || events.length === 0) {
      return;
    }

    const markerElements = this.$collectHydrationAttrElements(nodes)
      .filter((element) => element.hasAttribute('data-ev'));

    const eventCounts = readHydrationEventCounts(
      markerElements.map((element) => element.getAttribute('data-ev')),
      events.length,
    );
    if (!eventCounts) {
      throw new Error(
        `Hydration event markers for ${this.tagName.toLowerCase()} must use the count-based data-ev contract.`,
      );
    }

    let eventIndex = 0;
    for (let index = 0; index < markerElements.length; index += 1) {
      const target = markerElements[index];
      target.removeAttribute('data-ev');
      for (let offset = 0; offset < eventCounts[index]; offset += 1) {
        const entry = events[eventIndex];
        if (!entry) {
          break;
        }
        const [event, handler, needsEvent] = entry;
        this.$wire(target, event, handler, !!needsEvent);
        eventIndex += 1;
      }
    }
    if (eventIndex !== events.length) {
      throw new Error(
        `Hydration event marker count mismatch for ${this.tagName.toLowerCase()}: wired ${eventIndex} of ${events.length} events.`,
      );
    }
  }


  // ── Events ───────────────────────────────────────────────────────

  /**
   * Wire a single event on a specific target.
   *
   * Uses event delegation: a single listener per event type is installed
   * on the shadow root.  Each target stores its handler name in a
   * `data-eh-{event}` attribute.  When the event fires, the delegated
   * listener walks `e.target` up to find the attributed element and
   * dispatches to the named method.
   *
   * This eliminates per-listener closures entirely.  200 items × 5
   * events = 1000 bindings → 1 delegated listener per event type +
   * 1000 lightweight data attributes.  46% less heap than closures
   * with negligible dispatch overhead (~0.2µs per event).
   */
  private $delegatedEvents?: Map<string, boolean>;

  private $wire(
    target: EventTarget,
    eventName: string,
    handler: string,
    needsEvent: boolean,
  ): void {
    const method = (this as unknown as Record<string, Function>)[handler];
    if (typeof method !== 'function') return;

    // Root-level events (target === this) use a direct listener on the host.
    // The host element lives outside the shadow DOM so it cannot be reached
    // by the delegated-listener parentElement walk inside the shadow root.
    if (target === this || !(target instanceof Element)) {
      target.addEventListener(eventName, (e: Event) => {
        if (needsEvent) method.call(this, e);
        else method.call(this);
      });
      return;
    }

    target.setAttribute(`data-eh-${eventName}`, needsEvent ? `${handler}:e` : handler);

    // Install one delegated listener per event type on the shadow root
    if (!this.$delegatedEvents) {
      this.$delegatedEvents = new Map();
    }
    if (!this.$delegatedEvents.has(eventName)) {
      this.$delegatedEvents.set(eventName, true);
      this.shadowRoot?.addEventListener(eventName, (e: Event) => {
        let el = e.target as Element | null;
        while (el && el !== this) {
          const attr = el.getAttribute(`data-eh-${e.type}`);
          if (attr) {
            const needsE = attr.endsWith(':e');
            const name = needsE ? attr.slice(0, -2) : attr;
            const fn = (this as unknown as Record<string, Function>)[name];
            if (typeof fn === 'function') {
              if (needsE) fn.call(this, e);
              else fn.call(this);
            }
            return;
          }
          el = el.parentElement;
        }
      });
    }
  }

  // ── Conditionals (O(1) toggle) ───────────────────────────────────

  /**
   * Toggle a conditional block's visibility based on its condition expression.
   */
  private $toggleCond(c: CondBinding): void {
    const truthy = evaluateCondition(
      c.condition,
      (path, currentScope) => this.$resolveValue(path, currentScope),
      c.scope,
    );
    if (truthy) {
      if (!c.instance) {
        c.instance = this.$createBlockInstance(c.blockIndex, c.scope);
        const parent = c.anchor.parentNode;
        if (c.instance && parent instanceof Node) {
          this.$insertInstanceAfter(c.anchor, parent as ParentNode & Node, c.instance);
        }
      }

      if (c.instance) {
        this.$updateInstance(c.instance);
      }
      return;
    }

    if (c.instance) {
      this.$removeInstance(c.instance);
      c.instance = null;
    }
  }

  private $resolveValue(
    path: string,
    scope?: ScopeFrame,
  ): unknown {
    const scoped = this.$resolveScopeValue(path, scope);
    if (scoped !== undefined) {
      return scoped;
    }

    return this.$path(path);
  }

  private $resolveScopeValue(
    path: string,
    scope?: ScopeFrame,
  ): unknown {
    for (let frame = scope; frame; frame = frame.parent) {
      const resolved = resolveRepeatValue(frame.name, frame.value, path);
      if (resolved !== undefined) {
        return resolved;
      }
    }

    return undefined;
  }

  // ── Cleanup ──────────────────────────────────────────────────────

  /**
   * Remove SSR hydration markers from the shadow root.
   *
   * Strips one-shot hydration comments plus `data-w-*` attributes after
   * bindings are connected. Repeat boundary anchors that are still needed for
   * reconciliation are preserved. `data-ev` markers are removed individually
   * during event wiring.
   */
  private $clean(sr: ShadowRoot): void {
    // Remove SSR hydration markers (w-b:, w-r:) and data-w-* attrs
    const walker = document.createTreeWalker(sr, NodeFilter.SHOW_COMMENT);
    const keep = new Set<Comment>();
    if (this.$root) {
      this.$collectRepeatMarkers(this.$root, keep);
    }
    const rm: Comment[] = [];
    let c: Comment | null;
    while ((c = walker.nextNode() as Comment | null)) {
      if ((c.data.startsWith('w-b:') || c.data.startsWith('w-r:')) && !keep.has(c)) {
        rm.push(c);
      }
    }
    for (const n of rm) n.remove();

    for (const el of Array.from(sr.querySelectorAll('*'))) {
      for (const a of Array.from(el.attributes)) {
        if (a.name.startsWith('data-w-')) el.removeAttribute(a.name);
      }
    }
  }

  private $collectRepeatMarkers(instance: TemplateInstance, keep: Set<Comment>): void {
    for (const rep of instance.repeats) {
      if (rep.start) keep.add(rep.start);
      if (rep.end) keep.add(rep.end);
      for (const item of rep.instances) {
        this.$collectRepeatMarkers(item.instance, keep);
      }
    }

    for (const cond of instance.conds) {
      if (cond.instance) {
        this.$collectRepeatMarkers(cond.instance, keep);
      }
    }
  }

  // ── Path resolution ──────────────────────────────────────────────

  private $path(path: string): unknown {
    let cur: unknown = this;
    for (const k of path.split('.')) {
      if (cur == null) return undefined;
      cur = (cur as Record<string, unknown>)[k];
    }
    return cur;
  }
}
