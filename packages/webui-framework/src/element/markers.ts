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
  let depth = 1;
  let node: Node | null = repeatStart.nextSibling;
  while (node) {
    if (node.nodeType === 8 /* COMMENT_NODE */) {
      const data = (node as Comment).data;
      if (data === MARKER_REPEAT_START) {
        depth++;
      } else if (data === MARKER_REPEAT_END) {
        depth--;
        if (depth === 0) {
          end = node as Comment;
          break;
        }
      } else if (data === MARKER_REPEAT_ITEM && depth === 1) {
        items.push(node as Comment);
      }
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
 * Skip a complete structural block range and return the node that follows it.
 *
 * `start` must be a `<!--wc-->` or `<!--wr-->` comment.  Walks forward with
 * depth tracking over same-type markers so nested blocks of the same kind are
 * consumed as part of the range, and returns the sibling after the matching
 * closing marker (or `null` when the range is unterminated).
 */
export function skipBlockRange(start: Comment, data: string): ChildNode | null {
  const endTag = data === MARKER_COND_START ? MARKER_COND_END : MARKER_REPEAT_END;
  let depth = 1;
  let node: ChildNode | null = start.nextSibling;
  while (node && depth > 0) {
    if (node.nodeType === 8 /* COMMENT_NODE */) {
      const d = (node as Comment).data;
      if (d === data) depth++;
      else if (d === endTag) depth--;
    }
    if (depth > 0) node = node.nextSibling;
  }
  // Advance past the closing marker itself
  return node ? node.nextSibling : null;
}

/**
 * A resolved view of one SSR subtree, built in a single pre-order pass.
 *
 * `elements` maps each template element to the SSR element it hydrates, and
 * `conds` / `repeats` list the structural markers in document order.
 */
export interface SSRIndex {
  elements: Map<Node, Node>;
  conds: Comment[];
  repeats: Comment[];
}

/**
 * Pair a template subtree with its server-rendered counterpart in one walk.
 *
 * Resolving each binding independently costs O(bindings × width) because every
 * lookup rescans its parent's children from the start.  The compiler emits
 * bindings in source order and the server renders in source order, so a single
 * pre-order walk can resolve every target at once and hand out block markers in
 * the order the compiled `c` / `r` tables expect.
 *
 * Two properties of the SSR output shape the pairing:
 *
 *  - The renderer drops inter-element whitespace that `meta.h` keeps, so text
 *    nodes do **not** line up positionally.  Only elements are paired here.
 *  - Structural blocks render as `<!--wc-->`…`<!--/wc-->` / `<!--wr-->`…
 *    `<!--/wr-->` ranges that the template does not contain, so those ranges
 *    are skipped whole — their contents belong to the block's own metadata.
 *
 * The walk descends only where the template has children, which keeps it inside
 * the component that owns the template: a child component contributes no
 * children to `meta.h`, so its markers are never mistaken for this section's.
 *
 * Iterative by design — templates nest arbitrarily deep and recursion would put
 * that on the call stack.
 */
export function buildSSRIndex(tplRoot: Node, ssrRoot: Node, needMarkers: boolean): SSRIndex {
  const elements = new Map<Node, Node>();
  const conds: Comment[] = [];
  const repeats: Comment[] = [];
  const stack: Array<{ t: ChildNode | null; s: ChildNode | null }> = [
    { t: tplRoot.firstChild, s: ssrRoot.firstChild },
  ];

  while (stack.length > 0) {
    const frame = stack[stack.length - 1];

    // Advance the SSR cursor to the next element, recording block markers in
    // document order and stepping over their ranges.
    let s = frame.s;
    while (s) {
      const type = s.nodeType;
      if (type === 8 /* COMMENT_NODE */) {
        const data = (s as Comment).data;
        if (data === MARKER_COND_START) {
          conds.push(s as Comment);
          s = skipBlockRange(s as Comment, data);
          continue;
        }
        if (data === MARKER_REPEAT_START) {
          repeats.push(s as Comment);
          s = skipBlockRange(s as Comment, data);
          continue;
        }
      } else if (type === 1 /* ELEMENT_NODE */) {
        break;
      }
      s = s.nextSibling;
    }

    // Advance the template cursor to the next element.
    let t = frame.t;
    while (t && t.nodeType !== 1 /* ELEMENT_NODE */) t = t.nextSibling;

    if (!t || !s) {
      stack.pop();
      continue;
    }

    elements.set(t, s);
    frame.t = t.nextSibling;
    frame.s = s.nextSibling;

    // Descend when there is anything of ours down there.  Template children
    // always qualify.  An element with none can still hold block markers —
    // `<ul><for …></ul>` compiles to an empty `<ul>` — so it is worth walking
    // too, but only when this section actually has blocks to place.  A custom
    // element is the exception either way: with no template children it
    // contributes no slotted content, and whatever the server rendered inside
    // belongs to that component, not to this one.
    if (t.firstChild) {
      stack.push({ t: t.firstChild, s: s.firstChild });
    } else if (needMarkers && s.firstChild && (t as Element).tagName.indexOf('-') < 0) {
      stack.push({ t: null, s: s.firstChild });
    }
  }

  return { elements, conds, repeats };
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
        child = skipBlockRange(child as Comment, data);
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
