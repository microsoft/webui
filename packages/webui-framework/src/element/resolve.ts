// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * DOM resolution utilities for WebUIElement.
 *
 * Provides two resolution strategies:
 *
 * 1. **Client-created** ({@link resolve}) — walks childNode indices directly.
 *    The DOM matches the compiled template HTML exactly.
 *
 * 2. **SSR hydration** ({@link resolveSSR}) — walks SSR DOM in parallel with
 *    the compiled template DOM, using element/text ordinals to skip
 *    server-injected structural markers (`<!--wc-->`, `<!--wr-->`, etc.).
 *
 * Also provides SSR-specific helpers for marker traversal, text-node
 * lookup, and root-tag extraction.
 *
 * All functions are pure — they operate on DOM nodes and module-level
 * caches without accessing component instance state.
 */

import type { TemplateBlockMeta, TemplateNodePath } from '../template.js';
import { findByOrdinal } from './markers.js';

// ── Caches ──────────────────────────────────────────────────────

/** Parsed template DOM for SSR path mapping, keyed by TemplateBlockMeta. */
const templateDOMCache = new WeakMap<TemplateBlockMeta, Element>();

/** Cached root tag name extracted from meta.h. */
const rootTagCache = new WeakMap<TemplateBlockMeta, string | null>();

/**
 * Pre-computed ordinals for template nodes: childIndex → [nodeType, ordinal].
 * Avoids re-counting element/text siblings on every resolveSSR call.
 */
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

// ── Helpers ─────────────────────────────────────────────────────

/** Snapshot child nodes into a pre-allocated array. */
export function childNodesArray(parent: Node): Node[] {
  const children = parent.childNodes;
  const len = children.length;
  const result = new Array<Node>(len);
  for (let i = 0; i < len; i++) result[i] = children[i];
  return result;
}

/** Parse template HTML into a cached container element for SSR path mapping. */
export function getTemplateDom(meta: TemplateBlockMeta): Element {
  let cached = templateDOMCache.get(meta);
  if (cached) return cached;
  const div = document.createElement('div');
  div.innerHTML = meta.h;
  templateDOMCache.set(meta, div);
  return div;
}

// ── Client-created DOM resolution ───────────────────────────────

/**
 * Resolve a compiled node path against client-created DOM.
 *
 * Compiled paths are childNode indices into `meta.h` parsed by the browser.
 * For client-created components the DOM matches `meta.h` exactly, so
 * direct childNode indexing works.
 *
 * @param pathStart - Skip leading path segments for in-place block hydration.
 */
export function resolve(root: Node, path: TemplateNodePath, pathStart = 0): Node | null {
  let cur: Node = root;
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

// ── SSR hydration DOM resolution ────────────────────────────────

/**
 * Resolve a compiled node path against SSR-rendered DOM.
 *
 * The SSR DOM contains extra nodes injected by the server for rendered
 * structural blocks (conditionals and repeats). These shift element/text
 * ordinals relative to the template. This function walks SSR children
 * in parallel with the template DOM, using ordinal-based lookup
 * (via {@link findByOrdinal}) to skip structural block content.
 *
 * **Requires closing markers** (`<!--/wc-->`, `<!--/wr-->`) to still be
 * present — marker removal must be deferred until after all resolution.
 *
 * @param pathStart - Skip leading path segments for in-place block hydration.
 */
export function resolveSSR(ssrRoot: Node, tplRoot: Node, path: TemplateNodePath, pathStart = 0): Node | null {
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

// ── SSR helpers ─────────────────────────────────────────────────

/**
 * Find existing SSR text node by mapping template text-node ordinal.
 *
 * The SSR DOM may contain extra text nodes inside structural blocks
 * that are not in the compiled template. Skips marker-bounded ranges
 * to keep text ordinals aligned.
 */
export function findSSRText(ssrParent: Node, tplParent: Node, beforeIndex: number): Text | null {
  // Count how many text nodes precede `beforeIndex` in the template
  const ordinals = getTplOrdinals(tplParent);
  let textOrd = 0;
  for (let k = 0; k < beforeIndex; k++) {
    const entry = ordinals.get(k);
    if (entry && entry[0] === 3) textOrd++;
  }

  // Find the matching text node in SSR DOM, skipping structural block
  // content — same algorithm as resolveSSR (see findByOrdinal).
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

/** Find the next marker comment with the given data among a parent's children. */
export function findMarker(parent: Node, data: string, after?: Node | null): Comment | null {
  let child = after ? after.nextSibling : parent.firstChild;
  while (child) {
    if (child.nodeType === 8 && (child as Comment).data === data) {
      return child as Comment;
    }
    child = child.nextSibling;
  }
  return null;
}

/** Collect sibling nodes between a start marker and an end marker comment. */
export function collectBetween(start: Comment, endData: string): Node[] {
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
 * Check whether there is non-marker content between a conditional start
 * anchor and its closing marker.  Used during SSR hydration to detect
 * server-rendered conditional content even when the runtime condition value
 * has not been set yet (e.g. complex property from a parent repeat binding
 * that hydrates after its children).
 */
export function hasContentAfterMarker(anchor: Comment, endData: string): boolean {
  const sibling = anchor.nextSibling;
  if (!sibling) return false;
  // First non-marker sibling = content present.
  // First sibling matching endData = empty conditional.
  if (sibling.nodeType === 8 && (sibling as Comment).data === endData) {
    return false;
  }
  return true;
}

/** Extract root tag name from block metadata (cached). */
export function rootTag(meta: TemplateBlockMeta): string | null {
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
