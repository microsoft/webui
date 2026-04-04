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
      if (data === MARKER_REPEAT_END || data === MARKER_REPEAT_ITEM) return null;
    }
    node = node.nextSibling;
  }
  return null;
}
