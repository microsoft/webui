// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Repeat/list reconciliation for the WebUI runtime.
 *
 * Handles the full lifecycle of `@for(item of collection)` blocks:
 *
 * 1. **SSR setup** (`setupRepeat`) — finds repeat boundaries in hydrated DOM
 *    using `w-r:*` / `w-b:start:*for-*` comment markers, reads existing
 *    children to reconstruct the collection's observable state, and creates
 *    the initial `RepeatBinding`.
 *
 * 2. **Reconciliation** (`syncRepeat`) — called on every `$update()` when
 *    the collection changes.  Routes to keyed reconciliation (preserves DOM
 *    nodes across reorder) or sequential reconciliation (positional) based
 *    on whether the repeat block's root element has attribute bindings that
 *    can serve as keys.
 *
 * 3. **DOM reading** (`readRepeatFromDOM`) — extracts item values from
 *    SSR-rendered children by reading text markers and attribute bindings,
 *    then writes the reconstructed array to the component's backing field.
 *
 * All functions take a `RepeatHost` context — a minimal interface satisfied
 * by `WebUIElement` — rather than depending on the class directly.  This
 * keeps the module independently testable and avoids circular imports.
 */

import type {
  CompiledAttrMeta,
  CompiledAttrPart,
  CompiledConditionExpr,
  TemplateBlockMeta,
  TemplateSlotPath,
} from '../template.js';
import { conditionUsesPath, deriveConditionSeed } from './conditions.js';
import { isParentNode, collectElements, collectComments } from './paths.js';
import { getObservableNames } from '../decorators.js';
import type {
  RepeatBinding,
  RepeatHost,
  RepeatItemInstance,
  ScopeFrame,
  TemplateInstance,
} from './types.js';

// ── Marker helpers ──────────────────────────────────────────────────

/**
 * Extract the logical name from an SSR block marker comment.
 *
 * Markers have the form `w-b:start:N:name` or `w-b:end:N:name` where N is a
 * numeric index and name identifies the block (e.g. `if-1`, `for-2`).
 * Returns the name portion, or null if the comment doesn't match the prefix.
 */
export function readBlockMarkerName(
  data: string,
  prefix: 'w-b:start:' | 'w-b:end:',
): string | null {
  if (!data.startsWith(prefix)) {
    return null;
  }

  const parts = data.slice(prefix.length).split(':');
  if (parts.length < 2) {
    return null;
  }

  return parts.slice(1).join(':');
}

/**
 * Find the `w-b:end:N:name` comment that closes a `w-b:start:N:name` block.
 *
 * Handles nesting: if the same block name appears again before its end marker,
 * the depth counter tracks it so we return the correct matching closer.
 */
export function findMatchingBlockEndIndex(
  comments: Comment[],
  startIndex: number,
  name: string,
): number {
  let depth = 0;
  for (let index = startIndex + 1; index < comments.length; index += 1) {
    const startName = readBlockMarkerName(comments[index].data, 'w-b:start:');
    if (startName === name) {
      depth += 1;
      continue;
    }

    const endName = readBlockMarkerName(comments[index].data, 'w-b:end:');
    if (endName !== name) {
      continue;
    }

    if (depth === 0) {
      return index;
    }

    depth -= 1;
  }

  return -1;
}

// ── Pure helpers ─────────────────────────────────────────────────────

/**
 * Strip the repeat item variable prefix from a binding path.
 *
 * `relativeRepeatPath('item.title', 'item')` → `'title'`
 * `relativeRepeatPath('item', 'item')` → `''` (the item itself)
 * `relativeRepeatPath('count', 'item')` → `null` (not scoped to this repeat)
 */
export function relativeRepeatPath(path: string, itemVar: string): string | null {
  if (path === itemVar) {
    return '';
  }

  if (path.startsWith(`${itemVar}.`)) {
    return path.slice(itemVar.length + 1);
  }

  return null;
}

function resolveItemPath(item: unknown, path: string): unknown {
  let value = item;
  for (const segment of path.split('.')) {
    if (value == null || typeof value !== 'object') {
      return undefined;
    }
    value = (value as Record<string, unknown>)[segment];
  }
  return value;
}

/**
 * Resolve a dotted path against a repeat scope variable.
 *
 * When a binding inside `@for(item of items)` references `item.title`,
 * this function looks up `title` on the current scope value.
 */
export function resolveRepeatValue(
  scopeVar: string,
  scope: unknown,
  path: string,
): unknown {
  if (path === scopeVar) {
    return scope;
  }

  if (!path.startsWith(`${scopeVar}.`)) {
    return undefined;
  }

  let value = scope;
  for (const segment of path.slice(scopeVar.length + 1).split('.')) {
    if (value == null || typeof value !== 'object') {
      return undefined;
    }
    value = (value as Record<string, unknown>)[segment];
  }
  return value;
}

function repeatItemKey(item: unknown, attrMap: Record<string, string>): string | null {
  const keyPath = Object.values(attrMap)[0];
  if (keyPath === undefined) {
    return null;
  }

  const keyValue = keyPath === '' ? item : resolveItemPath(item, keyPath);
  return keyValue != null ? String(keyValue) : '';
}

function assignSeedPath(target: Record<string, unknown>, path: string, value: unknown): void {
  const parts = path.split('.');
  let cursor = target;
  for (let index = 0; index < parts.length - 1; index += 1) {
    const key = parts[index];
    const next = cursor[key];
    if (next == null || Array.isArray(next) || typeof next !== 'object') {
      cursor[key] = {};
    }
    cursor = cursor[key] as Record<string, unknown>;
  }

  cursor[parts[parts.length - 1]] = value;
}

function parentNode(node: Node): (ParentNode & Node) | null {
  const parent = node.parentNode;
  return parent && isParentNode(parent) ? parent : null;
}

// ── Block metadata ──────────────────────────────────────────────────

const repeatBlockInfoCache = new WeakMap<RepeatHost, Map<string, {
  rootTag: string | null;
  attrMap: Record<string, string>;
  rootBindings: CompiledAttrMeta[];
}>>();

function getBlockInfoCache(host: RepeatHost): Map<string, {
  rootTag: string | null;
  attrMap: Record<string, string>;
  rootBindings: CompiledAttrMeta[];
}> {
  let cache = repeatBlockInfoCache.get(host);
  if (!cache) {
    cache = new Map();
    repeatBlockInfoCache.set(host, cache);
  }
  return cache;
}

/**
 * Cache and compute metadata for a repeat block's root element.
 *
 * Determines the tag name of repeated children (e.g. `todo-item`), builds
 * a map of attribute names → item property paths for keyed reconciliation,
 * and collects root-level attribute bindings for SSR state reading.
 */
export function repeatBlockMetadata(
  host: RepeatHost,
  blockIndex: number,
  itemVar: string,
): { rootTag: string | null; attrMap: Record<string, string>; rootBindings: CompiledAttrMeta[] } {
  const cache = getBlockInfoCache(host);
  const cacheKey = `${blockIndex}:${itemVar}`;
  const cached = cache.get(cacheKey);
  if (cached) {
    return cached;
  }

  const block = host.$block(blockIndex);
  if (!block) {
    const empty = { rootTag: null, attrMap: {}, rootBindings: [] };
    cache.set(cacheKey, empty);
    return empty;
  }

  const template = document.createElement('template');
  template.innerHTML = block.h;
  const root = template.content.firstElementChild;
  const attrMap: Record<string, string> = {};
  const rootBindings: CompiledAttrMeta[] = [];
  if (root && block.a) {
    if (block.ag?.length) {
      for (const [path, start, count] of block.ag) {
        if (path.length !== 1 || path[0] !== 0) {
          continue;
        }

        for (let index = start; index < start + count; index += 1) {
          const entry = block.a[index];
          if (entry) {
            rootBindings.push(entry);
          }
        }
      }
    } else {
      const bindingRange = host.$readAttrBindingRange(root);
      if (bindingRange) {
        const [start, count] = bindingRange;
        for (let index = start; index < start + count; index += 1) {
          const entry = block.a[index];
          if (entry) {
            rootBindings.push(entry);
          }
        }
      }
    }
  }

  for (const entry of rootBindings) {
    const [name, kind, payload] = entry;
    if (kind === 0) {
      const rp = relativeRepeatPath(payload, itemVar);
      if (rp != null) {
        attrMap[name] = rp;
      }
      continue;
    }

    if (kind === 3) {
      const dynamic = host.$singleDynamicAttrPart(payload);
      if (!dynamic || dynamic.prefix !== '' || dynamic.suffix !== '') {
        continue;
      }

      const rp = relativeRepeatPath(dynamic.path, itemVar);
      if (rp != null) {
        attrMap[name] = rp;
      }
    }
  }

  const info = { rootTag: root?.tagName.toLowerCase() ?? null, attrMap, rootBindings };
  cache.set(cacheKey, info);
  return info;
}

// ── Element discovery ───────────────────────────────────────────────

function directChildElements(
  container: ParentNode & Node,
  childTag: string,
): Element[] {
  const elements: Element[] = [];
  for (let node = container.firstChild; node; node = node.nextSibling) {
    if (node instanceof Element && node.matches(childTag)) {
      elements.push(node);
    }
  }
  return elements;
}

/**
 * Find repeated child elements within a container, respecting boundary markers.
 *
 * Three modes:
 * - **Bounded** (start + end markers): collect elements between the markers.
 * - **Start-only**: collect consecutive matching siblings after the start marker.
 * - **Unbounded**: collect all matching children from the container.
 */
export function repeatElements(
  host: RepeatHost,
  container: ParentNode & Node,
  rep: RepeatBinding,
  childTag: string,
  expectedCount?: number,
): Element[] {
  if (rep.start && rep.end) {
    const elements: Element[] = [];
    for (let node = rep.start.nextSibling; node && node !== rep.end; node = node.nextSibling) {
      if (node instanceof Element && node.matches(childTag)) {
        elements.push(node);
      }
    }

    if (expectedCount && expectedCount > elements.length) {
      const dc = directChildElements(container, childTag);
      const firstSelected = elements[0] ?? null;
      const firstIndex = firstSelected ? dc.indexOf(firstSelected) : -1;
      if (dc.length >= expectedCount && firstIndex !== -1) {
        const startIndex = Math.max(0, Math.min(firstIndex, dc.length - expectedCount));
        return dc.slice(startIndex, startIndex + expectedCount);
      }
    }

    if (expectedCount && expectedCount > elements.length) {
      for (let node = rep.start.previousSibling; node; node = node.previousSibling) {
        if (node instanceof Element) {
          if (!node.matches(childTag)) break;
          elements.unshift(node);
          if (elements.length >= expectedCount) break;
          continue;
        }
        if (node instanceof Text && node.data.trim()) break;
      }
    }

    if (expectedCount && expectedCount > elements.length) {
      for (let node = rep.end.nextSibling; node; node = node.nextSibling) {
        if (node instanceof Element) {
          if (!node.matches(childTag)) break;
          elements.push(node);
          if (elements.length >= expectedCount) break;
          continue;
        }
        if (node instanceof Text && node.data.trim()) break;
      }
    }

    return elements;
  }

  if (rep.start) {
    const elements: Element[] = [];
    for (let node = rep.start.nextSibling; node; node = node.nextSibling) {
      if (node instanceof Element) {
        if (!node.matches(childTag)) break;
        elements.push(node);
        continue;
      }
      if (node instanceof Text && node.data.trim()) break;
    }
    return elements;
  }

  const elements = collectElements(Array.from(container.childNodes))
    .filter((element) => element.matches(childTag));
  if (expectedCount && expectedCount > 0 && elements.length > expectedCount) {
    return elements.slice(-expectedCount);
  }

  return elements;
}

// ── SSR reading ─────────────────────────────────────────────────────

function readRepeatTextBindings(el: Element, itemVar: string): Map<string, unknown> {
  const values = new Map<string, unknown>();
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_COMMENT);
  let repeatDepth = 0;
  let node: Comment | null;

  while ((node = walker.nextNode() as Comment | null)) {
    if (node.data.startsWith('w-r:start:')) { repeatDepth += 1; continue; }
    if (node.data.startsWith('w-r:end:')) { repeatDepth -= 1; continue; }

    if (repeatDepth > 0 || !node.data.startsWith('w-b:start:')) continue;

    const parts = node.data.slice('w-b:start:'.length).split(':');
    if (parts.length < 2) continue;

    const rp = relativeRepeatPath(parts.slice(1).join(':'), itemVar);
    if (rp == null) continue;

    const text = node.nextSibling;
    if (text instanceof Text) {
      values.set(rp, text.textContent ?? '');
    }
  }

  return values;
}

function readRepeatAttrBinding(
  host: RepeatHost,
  values: Map<string, unknown>,
  element: Element,
  entry: CompiledAttrMeta,
  itemVar: string,
): void {
  const [name, kind, payload] = entry;

  if (kind === 0) {
    const rp = relativeRepeatPath(payload, itemVar);
    if (rp == null) return;
    const raw = element.getAttribute(name);
    if (raw != null) values.set(rp, raw);
    return;
  }

  if (kind === 2) {
    seedRepeatConditionValue(values, payload, element.hasAttribute(name), itemVar);
    return;
  }

  if (kind === 3) {
    const dynamic = host.$singleDynamicAttrPart(payload);
    if (!dynamic) return;
    const rp = relativeRepeatPath(dynamic.path, itemVar);
    if (rp == null) return;
    const raw = element.getAttribute(name);
    if (raw == null) return;
    const value = host.$stripAffixes(raw, dynamic.prefix, dynamic.suffix);
    if (value !== undefined) values.set(rp, value);
  }
}

function readRepeatAttrBindings(
  host: RepeatHost,
  values: Map<string, unknown>,
  root: Element,
  block: TemplateBlockMeta,
  itemVar: string,
  rootBindings: CompiledAttrMeta[],
): void {
  if (!block.a && rootBindings.length === 0) return;

  let sawBindingMarkers = false;
  for (const element of collectElements([root])) {
    const bindingRange = host.$readAttrBindingRange(element);
    if (!bindingRange) continue;

    sawBindingMarkers = true;
    const [start, count] = bindingRange;
    for (let index = start; index < start + count; index += 1) {
      const entry = block.a?.[index];
      if (entry) readRepeatAttrBinding(host, values, element, entry, itemVar);
    }
  }

  if (!sawBindingMarkers) {
    for (const entry of rootBindings) {
      readRepeatAttrBinding(host, values, root, entry, itemVar);
    }
  }
}

function seedRepeatConditionValue(
  values: Map<string, unknown>,
  expr: CompiledConditionExpr,
  shown: boolean,
  itemVar: string,
): void {
  const seed = deriveConditionSeed(expr, shown);
  if (!seed || seed.kind !== 'path') return;

  const rp = relativeRepeatPath(seed.path, itemVar);
  if (rp == null) return;

  values.set(rp, seed.value);
}

function materializeRepeatItem(
  values: Map<string, unknown>,
): unknown {
  if (values.size === 0) return undefined;

  const objectKeys = Array.from(values.keys()).filter((key) => key.length > 0);
  if (objectKeys.length === 0) return values.get('') ?? '';

  const item: Record<string, unknown> = {};
  for (const [path, value] of values) {
    if (!path) continue;
    assignSeedPath(item, path, value);
  }

  return item;
}

// ── Block analysis ──────────────────────────────────────────────────

function attrBindingUsesPath(entry: CompiledAttrMeta, path: string): boolean {
  const [, kind, payload] = entry;
  if (kind === 0 || kind === 1) return payload === path;
  if (kind === 2) return conditionUsesPath(payload, path);
  return payload.some((part: CompiledAttrPart) => typeof part !== 'string' && part[0] === path);
}

/**
 * Check whether any binding in a block or its nested blocks references `path`.
 *
 * Used to determine whether a repeat block needs a particular property
 * (e.g. to decide if the block template uses `item.activeClass`).
 */
export function blockUsesPath(
  host: RepeatHost,
  block: TemplateBlockMeta,
  path: string,
  visited = new Set<number>(),
): boolean {
  if (block.t?.includes(path)) return true;

  if (block.tx?.some(([, parts]) => parts.some((part) => typeof part !== 'string' && part[0] === path))) {
    return true;
  }

  if (block.a?.some((entry) => attrBindingUsesPath(entry, path))) return true;

  if (block.c?.some(([condition]) => conditionUsesPath(condition, path))) return true;

  for (const [, blockIndex] of block.c ?? []) {
    if (visited.has(blockIndex)) continue;
    visited.add(blockIndex);
    const nested = host.$block(blockIndex);
    if (nested && blockUsesPath(host, nested, path, visited)) return true;
  }

  for (const [collection, , blockIndex] of block.r ?? []) {
    if (collection === path) return true;
    if (visited.has(blockIndex)) continue;
    visited.add(blockIndex);
    const nested = host.$block(blockIndex);
    if (nested && blockUsesPath(host, nested, path, visited)) return true;
  }

  return false;
}

// ── Collection assignment ───────────────────────────────────────────

/**
 * Write a reconstructed collection array back to the component's backing field.
 *
 * Handles both flat paths (`'items'`) and scoped paths inside nested repeats.
 * Uses the `_name` backing key for `@observable` properties to avoid
 * triggering a reactive update during hydration.
 */
export function assignCollectionValue(
  host: RepeatHost,
  hostObj: Record<string, unknown>,
  hostCtor: Function,
  path: string,
  items: unknown[],
  scope?: ScopeFrame,
): void {
  for (let frame = scope; frame; frame = frame.parent) {
    const rp = relativeRepeatPath(path, frame.name);
    if (rp == null) continue;

    if (rp === '') {
      frame.value = items;
      return;
    }

    if (frame.value != null && typeof frame.value === 'object' && !Array.isArray(frame.value)) {
      assignSeedPath(frame.value as Record<string, unknown>, rp, items);
    }
    return;
  }

  const parts = path.split('.');
  const root = parts[0];
  const key = getObservableNames(hostCtor).has(root) ? `_${root}` : root;
  if (parts.length === 1) {
    hostObj[key] = items;
    return;
  }

  let target = hostObj[key];
  if (target == null || Array.isArray(target) || typeof target !== 'object') {
    target = {};
    hostObj[key] = target;
  }
  assignSeedPath(target as Record<string, unknown>, parts.slice(1).join('.'), items);
}

// ── SSR setup ───────────────────────────────────────────────────────

/**
 * Read initial array state from SSR-rendered child elements.
 *
 * Walks existing children (e.g. `<todo-item title="Buy milk">`) and
 * extracts their attribute/text values to reconstruct the `@observable`
 * collection so that `this.items` reflects the server-rendered state.
 */
export function readRepeatFromDOM(
  host: RepeatHost,
  hostObj: Record<string, unknown>,
  hostCtor: Function,
  rep: RepeatBinding,
  container: ParentNode & Node,
): void {
  if (!rep.rootTag) return;

  const block = host.$block(rep.blockIndex);
  if (!block) return;

  const elements = repeatElements(host, container, rep, rep.rootTag);
  if (elements.length === 0) {
    assignCollectionValue(host, hostObj, hostCtor, rep.collection, [], rep.scope);
    rep.synced = true;
    rep.instances = [];
    return;
  }

  const items: unknown[] = [];
  const instances: RepeatItemInstance[] = [];
  for (const el of elements) {
    const values = new Map<string, unknown>();

    for (const [path, value] of readRepeatTextBindings(el, rep.itemVar)) {
      values.set(path, value);
    }

    readRepeatAttrBindings(host, values, el, block, rep.itemVar, rep.rootBindings);

    const item = materializeRepeatItem(values);
    if (item !== undefined) {
      items.push(item);
      const itemScope: ScopeFrame = {
        name: rep.itemVar,
        value: item,
        parent: rep.scope,
      };
      const instance = host.$hydrateExistingBlockInstance(rep.blockIndex, [el], itemScope);
      if (instance) {
        instances.push({
          key: repeatItemKey(item, rep.attrMap),
          value: item,
          instance,
        });
      }
    }
  }

  if (items.length > 0) {
    assignCollectionValue(host, hostObj, hostCtor, rep.collection, items, rep.scope);
    rep.synced = true;
    rep.instances = instances;
  }
}

/**
 * Set up an SSR-backed repeat binding from hydration markers.
 *
 * The handler emits two marker layers for each for-loop:
 * - `w-b:start:N:for-M` / `w-b:end:N:for-M` — outer block boundary
 * - `w-r:start:I` / `w-r:end:I` — per-item iteration markers
 *
 * This function finds the outer block boundary, discovers the container,
 * reads initial state from SSR children, and pushes a `RepeatBinding`
 * onto the owning template instance.
 *
 * Client-created components bypass this function entirely — they create
 * repeat bindings directly in `$createClientTemplateInstance` using
 * slot locators from compiled metadata.
 */
export function setupRepeat(
  host: RepeatHost,
  hostObj: Record<string, unknown>,
  hostCtor: Function,
  nodes: Node[],
  owner: TemplateInstance,
  markerId: number,
  collection: string,
  itemVar: string,
  blockIndex: number,
  scope?: ScopeFrame,
  _meta?: TemplateBlockMeta,
  _slotPath?: TemplateSlotPath,
): void {
  const comments = collectComments(nodes);
  let container: (ParentNode & Node) | null = null;
  let loopStart: Comment | null = null;
  let loopEnd: Comment | null = null;

  // SSR emits globally-scoped for-loop marker IDs (for-8, for-9, for-10)
  // while setup calls use local repeat indices (0, 1, 2). Re-map by
  // finding the Nth for-* start marker at the top repeat depth.
  let forStartIndex = 0;
  let targetForName: string | null = null;
  let rDepth = 0;
  for (const c of comments) {
    const d = c.data;
    if (d.startsWith('w-r:start:')) { rDepth++; continue; }
    if (d.startsWith('w-r:end:')) { rDepth--; continue; }
    if (rDepth > 0) continue;

    if (d.startsWith('w-b:start:')) {
      const parts = d.slice('w-b:start:'.length).split(':');
      if (parts.length >= 2) {
        const name = parts.slice(1).join(':');
        if (name.startsWith('for-')) {
          if (forStartIndex === markerId) {
            targetForName = name;
            loopStart = c;
            container = parentNode(c);
          }
          forStartIndex++;
        }
      }
    }
    if (d.startsWith('w-b:end:') && targetForName) {
      const parts = d.slice('w-b:end:'.length).split(':');
      if (parts.length >= 2) {
        const name = parts.slice(1).join(':');
        if (name === targetForName) {
          loopEnd = c;
          break;
        }
      }
    }
  }

  const { rootTag, attrMap, rootBindings } = repeatBlockMetadata(host, blockIndex, itemVar);

  // If no explicit block markers, find the container from the first
  // matching child element (handles SSR HTML without w-b markers).
  if (!container && rootTag) {
    const first = collectElements(nodes).find((element) => element.matches(rootTag));
    container = first ? parentNode(first) : null;
  }

  const normalizedStart = container && loopStart?.parentNode !== container ? null : loopStart;
  const normalizedEnd = container && loopEnd?.parentNode !== container ? null : loopEnd;

  const rep: RepeatBinding = {
    markerId,
    collection,
    itemVar,
    blockIndex,
    container,
    start: normalizedStart,
    end: normalizedEnd,
    scope,
    owner,
    instances: [],
    rootTag,
    attrMap,
    rootBindings,
  };

  if (container && rootTag) {
    readRepeatFromDOM(host, hostObj, hostCtor, rep, container);
  }

  owner.repeats.push(rep);
}

// ── Reconciliation ──────────────────────────────────────────────────

function createRepeatItemInstance(
  host: RepeatHost,
  rep: RepeatBinding,
  item: unknown,
): RepeatItemInstance | null {
  const scope: ScopeFrame = {
    name: rep.itemVar,
    value: item,
    parent: rep.scope,
  };
  const instance = host.$createBlockInstance(rep.blockIndex, scope);
  if (!instance) return null;

  return {
    key: repeatItemKey(item, rep.attrMap),
    value: item,
    instance,
  };
}

function syncSequentialRepeat(
  host: RepeatHost,
  rep: RepeatBinding,
  items: unknown[],
  container: ParentNode & Node,
): void {
  const next: RepeatItemInstance[] = [];
  for (let index = 0; index < items.length; index += 1) {
    const item = items[index];
    const entry = rep.instances[index] ?? createRepeatItemInstance(host, rep, item);
    if (!entry) continue;

    entry.value = item;
    entry.key = repeatItemKey(item, rep.attrMap);
    if (entry.instance.scope) entry.instance.scope.value = item;
    next.push(entry);
  }

  for (let index = items.length; index < rep.instances.length; index += 1) {
    host.$removeInstance(rep.instances[index].instance);
  }
  rep.instances = next;

  let cursor: Node | null = rep.start;
  for (const entry of rep.instances) {
    cursor = host.$insertInstanceAfter(cursor, container, entry.instance);
  }

  for (const entry of rep.instances) {
    host.$updateInstance(entry.instance);
  }
}

function syncKeyedRepeat(
  host: RepeatHost,
  rep: RepeatBinding,
  items: unknown[],
  container: ParentNode & Node,
): void {
  const existing = new Map<string, RepeatItemInstance>();
  for (const entry of rep.instances) {
    if (entry.key != null) existing.set(entry.key, entry);
  }

  const next: RepeatItemInstance[] = [];
  for (const item of items) {
    const key = repeatItemKey(item, rep.attrMap);
    const entry = key != null ? existing.get(key) : undefined;
    if (entry && key != null) {
      existing.delete(key);
      entry.value = item;
      entry.key = key;
      if (entry.instance.scope) entry.instance.scope.value = item;
      next.push(entry);
      continue;
    }

    const created = createRepeatItemInstance(host, rep, item);
    if (created) next.push(created);
  }

  for (const leftover of existing.values()) {
    host.$removeInstance(leftover.instance);
  }
  rep.instances = next;

  let cursor: Node | null = rep.start;
  for (const entry of rep.instances) {
    cursor = host.$insertInstanceAfter(cursor, container, entry.instance);
  }

  for (const entry of rep.instances) {
    host.$updateInstance(entry.instance);
  }
}

/**
 * Reconcile a repeat binding against its current collection value.
 *
 * Called by `$updateInstance` on every reactive update.  Resolves the
 * collection path, diffs against existing instances, and routes to
 * keyed or sequential reconciliation.  Keyed mode preserves DOM nodes
 * across reorders; sequential mode is simpler and positional.
 */
export function syncRepeat(
  host: RepeatHost,
  hostObj: Record<string, unknown>,
  hostCtor: Function,
  rep: RepeatBinding,
): void {
  const resolved = host.$resolveValue(rep.collection, rep.scope);
  const items = Array.isArray(resolved) ? resolved : [];

  let container = rep.container
    ?? (rep.start ? parentNode(rep.start) : null)
    ?? (rep.owner.nodes[0] ? parentNode(rep.owner.nodes[0]) : null);
  if (!container || !rep.rootTag) return;

  rep.container = container;
  const existingRoots = repeatElements(host, container, rep, rep.rootTag, items.length);

  if (!rep.synced && items.length === 0 && (existingRoots.length > 0 || rep.instances.length > 0)) {
    return;
  }

  if (rep.instances.length === 0 && existingRoots.length > 0) {
    if (items.length === existingRoots.length) {
      for (let index = 0; index < items.length; index += 1) {
        const itemScope: ScopeFrame = {
          name: rep.itemVar,
          value: items[index],
          parent: rep.scope,
        };
        const instance = host.$hydrateExistingBlockInstance(rep.blockIndex, [existingRoots[index]], itemScope);
        if (instance) {
          rep.instances.push({
            key: repeatItemKey(items[index], rep.attrMap),
            value: items[index],
            instance,
          });
        }
      }
    } else {
      for (const element of existingRoots) element.remove();
    }
  }

  rep.synced = true;
  if (Object.keys(rep.attrMap).length > 0) {
    syncKeyedRepeat(host, rep, items, container);
    return;
  }

  syncSequentialRepeat(host, rep, items, container);
}
