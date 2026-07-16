// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Hydration marker utilities for zero-DOM-mutation in-place hydration.
 *
 * The WebUI handler plugin emits lightweight HTML comment markers around
 * structural boundaries (for-loops and if-conditions). These utilities
 * walk markers and locate elements so the hydration path can wire
 * bindings in-place without reparenting DOM nodes.
 *
 * Marker format:
 *   <!--wr-->   repeat block start
 *   <!--/wr-->  repeat block end
 *   <!--wi-->   repeat item boundary
 *   <!--wc-->   conditional block start
 *   <!--/wc-->  conditional block end
 */

// Marker data constants matching the handler plugin output.
export const MARKER_REPEAT_START = 'wr';
export const MARKER_REPEAT_END = '/wr';
export const MARKER_COND_START = 'wc';
export const MARKER_COND_END = '/wc';

const MARKER_REPEAT_ITEM = 'wi';

/**
 * Collect the item markers (<!--wi-->) within a repeat range.
 *
 * Walks siblings from the repeat start marker to the repeat end marker.
 * Returns an array of <!--wi--> comment nodes that delineate items.
 */
export function collectItemMarkers(repeatStart: Comment): { items: Comment[]; end: Comment | null } {
  const items: Comment[] = [];
  let end: Comment | null = null;
  let node: Node | null = repeatStart.nextSibling;
  while (node) {
    if (node.nodeType === 8 /* COMMENT_NODE */) {
      const data = (node as Comment).data;
      if (data === MARKER_REPEAT_END) { end = node as Comment; break; }
      if (data === MARKER_REPEAT_ITEM) items.push(node as Comment);
    }
    node = node.nextSibling;
  }
  return { items, end };
}

/**
 * Get the next element sibling after a marker comment, skipping
 * whitespace text nodes and other comments.
 */
export function nextElement(marker: Comment): Element | null {
  let node: Node | null = marker.nextSibling;
  while (node) {
    if (node.nodeType === 1 /* ELEMENT_NODE */) return node as Element;
    if (node.nodeType === 8 /* COMMENT_NODE */) {
      const data = (node as Comment).data;
      if (data === MARKER_REPEAT_END || data === MARKER_REPEAT_ITEM || data === MARKER_COND_END) return null;
    }
    node = node.nextSibling;
  }
  return null;
}

/**
 * Find the Nth child of a given nodeType, skipping structural block ranges.
 *
 * The compiled template static HTML (`meta.h`) does not contain conditional
 * or repeat block content — those are stored as separate block metadata.
 * But the SSR DOM has this content rendered inline between marker pairs
 * (`<!--wc-->...<!--/wc-->` and `<!--wr-->...<!--/wr-->`).
 *
 * This function walks `parent.firstChild` → siblings, counting only
 * children of the requested `nodeType` that are NOT inside a structural
 * block range.  Nested blocks of the same type are handled via depth
 * tracking.  Returns the child at the given `ordinal`, or null.
 *
 * Used by `$resolveSSR` (element ordinals) and `$findSSRText` (text
 * ordinals) to keep SSR DOM ordinals aligned with template metadata.
 *
 * **Requires closing markers to still be in the DOM** — caller must
 * not remove `<!--/wc-->` or `<!--/wr-->` before all resolution is done.
 */
export function findByOrdinal(parent: Node, nodeType: number, ordinal: number): Node | null {
  let count = 0;
  let child = parent.firstChild;
  while (child) {
    // Detect a structural block opening marker and skip the entire range.
    if (child.nodeType === 8 /* COMMENT_NODE */) {
      const data = (child as Comment).data;
      if (data === MARKER_COND_START || data === MARKER_REPEAT_START) {
        const endTag = data === MARKER_COND_START ? MARKER_COND_END : MARKER_REPEAT_END;
        let depth = 1;
        child = child.nextSibling;
        while (child && depth > 0) {
          if (child.nodeType === 8 /* COMMENT_NODE */) {
            const d = (child as Comment).data;
            if (d === data) depth++;
            else if (d === endTag) depth--;
          }
          if (depth > 0) child = child.nextSibling;
        }
        // Advance past the closing marker itself
        if (child) child = child.nextSibling;
        continue;
      }
    }
    if (child.nodeType === nodeType) {
      if (count === ordinal) return child;
      count++;
    }
    child = child.nextSibling;
  }
  return null;
}
