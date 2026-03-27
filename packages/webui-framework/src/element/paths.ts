// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * DOM path resolution and node collection utilities.
 *
 * The Rust compiler emits compiled template metadata that references DOM nodes
 * by numeric child-index paths (e.g. `[0, 2, 1]` = "root's first child → its
 * third child → its second child").  These functions resolve those paths
 * against live DOM trees — both the clean compiled HTML and the SSR-hydrated
 * HTML that may contain extra whitespace text nodes and comment markers.
 *
 * ## Why "aligned" resolution exists
 *
 * SSR HTML contains hydration markers (`<!--w-b:...-->`, `<!--w-r:...-->`)
 * and browser-inserted whitespace text nodes that shift child indices relative
 * to the compiled template's marker-free HTML.  The `resolveTemplateAligned*`
 * functions walk both trees in parallel, skipping insignificant nodes
 * (whitespace, markers) to find the correct live DOM node even when indices
 * don't match directly.
 *
 * ## Collection helpers
 *
 * `collectElements` and `collectComments` perform iterative depth-first
 * traversal (no recursion) to gather nodes from a shadow root's child list.
 * Used by the hydration walk, repeat setup, and cleanup pass.
 */

import type { TemplateNodePath, TemplateSlotPath } from '../template.js';
import type { ResolvedSlot } from './types.js';

/** Type guard: is this node a container that has child nodes? */
export function isParentNode(node: Node): node is ParentNode & Node {
  return node instanceof Element
    || node instanceof DocumentFragment
    || node instanceof ShadowRoot;
}

/**
 * Should this node be ignored when counting "significant" children?
 *
 * SSR HTML has whitespace-only text nodes and hydration comment markers
 * (`w-b:*`, `w-r:*`) that don't exist in compiled template HTML.
 * Skipping them lets aligned resolution match nodes by semantic position
 * rather than raw child index.
 */
export function isSkippablePathNode(node: Node): boolean {
  return (node instanceof Text && node.data.trim() === '')
    || (node instanceof Comment
      && (node.data.startsWith('w-b:') || node.data.startsWith('w-r:')));
}

/**
 * Does `element` match `reference` by tag name and all of reference's attributes?
 *
 * Used during SSR hydration to verify that a resolved node is the correct
 * element when child indices are shifted by markers/whitespace.
 */
export function matchesTemplateElement(element: Element, reference: Element): boolean {
  if (element.tagName !== reference.tagName) {
    return false;
  }

  for (const attr of Array.from(reference.attributes)) {
    if (element.getAttribute(attr.name) !== attr.value) {
      return false;
    }
  }

  return true;
}

/**
 * Walk a numeric child-index path from a known root.
 *
 * `path = [0, 2]` → `root.childNodes[0].childNodes[2]`.
 * Used for client-created DOM where the tree matches compiled HTML exactly.
 */
export function resolveNodePath(
  root: ParentNode & Node,
  path: TemplateNodePath,
): Node | null {
  let current: Node = root;
  for (const index of path) {
    if (!isParentNode(current)) {
      return null;
    }

    const next = current.childNodes.item(index);
    if (!next) {
      return null;
    }
    current = next;
  }

  return current;
}

/**
 * Walk a path against a flat list of top-level nodes (the shadow root's children).
 *
 * The first path segment indexes into `nodes[]` directly; subsequent segments
 * descend via `childNodes`.  An empty path returns the common parent of all nodes.
 * Used during SSR hydration where the shadow root's children are passed as an array.
 */
export function resolveNodePathFromNodes(nodes: Node[], path: TemplateNodePath): Node | null {
  if (path.length === 0) {
    const parent = nodes[0]?.parentNode ?? null;
    if (!parent) {
      return null;
    }

    for (const node of nodes) {
      if (node.parentNode !== parent) {
        return null;
      }
    }

    return parent;
  }

  let current = nodes[path[0]] ?? null;
  if (!current) {
    return null;
  }

  for (let index = 1; index < path.length; index += 1) {
    if (!isParentNode(current)) {
      return null;
    }

    const next = current.childNodes.item(path[index]);
    if (!next) {
      return null;
    }
    current = next;
  }

  return current;
}

/**
 * Count how many "significant" (non-skippable) nodes appear at or before `index`.
 *
 * Returns the 0-based ordinal among significant nodes.  This lets the aligned
 * resolver say "I want the 3rd real element" regardless of how many whitespace
 * text nodes or comment markers are interspersed.
 */
export function significantNodeOrdinal(nodes: readonly Node[], index: number): number | null {
  if (index < 0 || index >= nodes.length) {
    return null;
  }

  let ordinal = -1;
  for (let cursor = 0; cursor <= index; cursor += 1) {
    if (isSkippablePathNode(nodes[cursor])) {
      continue;
    }
    ordinal += 1;
  }

  return ordinal;
}

/**
 * Find the node at the Nth significant position (skipping whitespace/markers).
 *
 * Inverse of `significantNodeOrdinal`: given an ordinal computed from the
 * compiled template's children, find the corresponding node in live SSR DOM.
 */
export function nodeAtSignificantOrdinal(nodes: readonly Node[], ordinal: number): Node | null {
  if (ordinal < 0) {
    return null;
  }

  let current = -1;
  for (const node of nodes) {
    if (isSkippablePathNode(node)) {
      continue;
    }
    current += 1;
    if (current === ordinal) {
      return node;
    }
  }

  return null;
}

/**
 * Find the live DOM child that corresponds to `templateChildren[index]`.
 *
 * If the template child is skippable (whitespace/marker), uses the raw index.
 * Otherwise computes the significant ordinal from the template side, then
 * finds the node at that ordinal in the live DOM side.
 */
export function resolveAlignedChildNode(
  nodes: readonly Node[],
  templateChildren: readonly Node[],
  index: number,
): Node | null {
  const templateTarget = templateChildren[index];
  if (!templateTarget) {
    return null;
  }

  if (isSkippablePathNode(templateTarget)) {
    return nodes[index] ?? null;
  }

  const ordinal = significantNodeOrdinal(templateChildren, index);
  if (ordinal === null) {
    return null;
  }

  return nodeAtSignificantOrdinal(nodes, ordinal);
}

/**
 * Walk a path against SSR DOM using the compiled template as a guide.
 *
 * At each level, the function finds the correct live DOM child by comparing
 * significant-node positions between the template tree and the live tree.
 * This handles SSR HTML that has extra whitespace and hydration markers
 * shifting child indices relative to compiled template HTML.
 */
export function resolveTemplateAlignedNodePathFromNodes(
  nodes: readonly Node[],
  templateRoot: ParentNode & Node,
  path: TemplateNodePath,
): Node | null {
  if (path.length === 0) {
    return resolveNodePathFromNodes([...nodes], path);
  }

  let templateCurrent: Node = templateRoot;
  let currentNodes = nodes;
  let resolved: Node | null = null;

  for (const index of path) {
    if (!isParentNode(templateCurrent)) {
      return null;
    }

    const templateChildren = Array.from(templateCurrent.childNodes);
    const templateTarget = templateChildren[index];
    if (!templateTarget) {
      return null;
    }

    resolved = resolveAlignedChildNode(currentNodes, templateChildren, index);
    if (!resolved) {
      return null;
    }

    templateCurrent = templateTarget;
    currentNodes = isParentNode(resolved) ? Array.from(resolved.childNodes) : [];
  }

  return resolved;
}

/** Resolve a path and return the node only if it's an Element. */
export function resolveElementPath(
  root: ParentNode & Node,
  path: TemplateNodePath,
): Element | null {
  const node = resolveNodePath(root, path);
  return node instanceof Element ? node : null;
}

/**
 * Resolve a slot locator to a parent node and insertion reference.
 *
 * Slot paths `[parentPath, beforeIndex, order?]` tell the client runtime
 * where to insert dynamically created text nodes, conditional anchors,
 * or repeat anchors within compiled HTML.
 */
export function resolveSlotPath(
  root: ParentNode & Node,
  slotPath: TemplateSlotPath,
): ResolvedSlot | null {
  const [parentPath, beforeIndex, order = 0] = slotPath;
  const parentNode = resolveNodePath(root, parentPath);
  if (!parentNode || !isParentNode(parentNode)) {
    return null;
  }

  return {
    parent: parentNode,
    nextSibling: parentNode.childNodes.item(beforeIndex) ?? null,
    order,
  };
}

// ── DOM collection helpers ──────────────────────────────────────────

/**
 * Iterative depth-first collection of nodes matching a predicate.
 *
 * Uses a manual stack instead of recursion for performance in large
 * shadow trees (avoids call-stack depth and stays hot-path friendly).
 */
export function collectNodes<T extends Node>(
  nodes: readonly Node[],
  predicate: (node: Node) => node is T,
): T[] {
  const collected: T[] = [];
  const stack: Node[] = [];
  for (let index = nodes.length - 1; index >= 0; index -= 1) {
    stack.push(nodes[index]);
  }

  while (stack.length > 0) {
    const node = stack.pop()!;
    if (predicate(node)) {
      collected.push(node);
    }

    for (let child = node.lastChild; child; child = child.previousSibling) {
      stack.push(child);
    }
  }

  return collected;
}

/** Collect all Element nodes in a depth-first walk of `nodes` and their subtrees. */
export function collectElements(nodes: readonly Node[]): Element[] {
  return collectNodes(nodes, (node): node is Element => node instanceof Element);
}

/** Collect all Comment nodes in a depth-first walk of `nodes` and their subtrees. */
export function collectComments(nodes: readonly Node[]): Comment[] {
  return collectNodes(nodes, (node): node is Comment => node instanceof Comment);
}
