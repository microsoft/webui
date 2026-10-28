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
 *   <!--wN-->   raw HTML binding start
 *   <!--/wN-->  raw HTML binding end
 */

import { isComponentStyleResourceMarker } from './styles.js';

// Marker data constants matching the handler plugin output.
export const MARKER_REPEAT_START = 'wr';
export const MARKER_REPEAT_END = '/wr';
export const MARKER_COND_START = 'wc';
export const MARKER_COND_END = '/wc';
export const MARKER_RAW_START = 'w';
export const MARKER_RAW_END = '/w';

const MARKER_REPEAT_ITEM = 'wi';

/** Build the paired marker data for a client-created raw binding. */
export function rawMarker(index: number, closing = false): string {
  return `${closing ? MARKER_RAW_END : MARKER_RAW_START}${index}`;
}

function isRawStartMarker(data: string): boolean {
  if (data.length < 2 || data.charCodeAt(0) !== 119 /* w */) return false;
  for (let i = 1; i < data.length; i++) {
    const code = data.charCodeAt(i);
    if (code < 48 /* 0 */ || code > 57 /* 9 */) return false;
  }
  return true;
}

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
 * Find the closing marker of a complete structural block range.
 *
 * `start` must be a `<!--wc-->`, `<!--wr-->`, or `<!--wN-->` comment. Walks forward with
 * depth tracking over same-type markers so nested blocks of the same kind are
 * consumed as part of the range. Returns `null` when the range is unterminated.
 */
export function findBlockEnd(start: Comment, data: string): Comment | null {
  const endTag = data === MARKER_COND_START
    ? MARKER_COND_END
    : isRawStartMarker(data)
      ? `/${data}`
      : MARKER_REPEAT_END;
  let depth = 1;
  let node: ChildNode | null = start.nextSibling;
  while (node) {
    if (node.nodeType === 8 /* COMMENT_NODE */) {
      const d = (node as Comment).data;
      if (d === data) depth++;
      else if (d === endTag && --depth === 0) return node as Comment;
    }
    node = node.nextSibling;
  }
  return null;
}

/**
 * Skip a complete structural block range and return the node that follows it,
 * or `null` when the range is unterminated.
 */
export function skipBlockRange(start: Comment, data: string): ChildNode | null {
  const end = findBlockEnd(start, data);
  return end ? end.nextSibling : null;
}

/**
 * Return the node `index` siblings after `first`, stopping at `end`.
 *
 * Yields `end` (or `null` past the parent's last child) when the range is
 * shorter, which is the insertion point that appends to the range.
 */
export function rangeChildAt(first: ChildNode | null, end: Node | null, index: number): Node | null {
  let node: ChildNode | null = first;
  while (index > 0 && node && node !== end) {
    node = node.nextSibling;
    index--;
  }
  return node;
}

/**
 * Number a client-created subtree the way the compiler numbered the template.
 *
 * Cloned template DOM matches `h` node for node, so a plain pre-order walk
 * reproduces the compiled indices — no server markers are involved and nothing
 * needs to be skipped.  Index `0` is the section root.
 */
export function collectTemplateElements(root: Node): Array<Node | undefined> {
  const elements: Array<Node | undefined> = [root];
  const stack: Array<ChildNode | null> = [root.firstChild];

  while (stack.length > 0) {
    const node = stack.pop();
    let child = node ?? null;
    while (child) {
      if (child.nodeType === 1 /* ELEMENT_NODE */) {
        elements.push(child);
        if (child.firstChild) {
          // Depth first: finish this element's subtree before its siblings.
          stack.push(child.nextSibling);
          child = child.firstChild;
          continue;
        }
      }
      child = child.nextSibling;
    }
  }

  return elements;
}

/**
 * A resolved view of one SSR subtree, built in a single pre-order pass.
 *
 * `elements` holds the server-rendered element for each compiled pre-order
 * index. Block markers and raw HTML ranges are listed in document order.
 */
export interface SSRIndex {
  /** Server-rendered elements by pre-order index; `0` is the section root. */
  elements: Array<Node | undefined>;
  conds: Comment[];
  repeats: Comment[];
  raws: Array<[Comment, Comment]>;
}

/**
 * Pair compiled static topology with its server-rendered subtree in one walk.
 *
 * The compiler records each element's direct child count and ownership bit,
 * numbers every element of a section in pre-order, and addresses
 * bindings by that number.  Walking the server output in the same order rebuilds
 * the mapping in one pass, so each binding resolves by array index.  Block
 * markers come out in document order for the same reason, which is what lets
 * the compiled `c` / `r` tables be zipped against them positionally.
 *
 * Two properties of the SSR output shape the pairing:
 *
 *  - The renderer drops inter-element whitespace that `meta.h` keeps, so text
 *    nodes do **not** line up positionally.  Only elements are paired here.
 *  - Structural blocks render as `<!--wc-->`…`<!--/wc-->` / `<!--wr-->`…
 *    `<!--/wr-->` ranges that the template does not contain, so those ranges
 *    are skipped whole - their contents belong to the block's own metadata.
 *
 * A block section is hydrated in place: `ssrRoot` is its live parent and the
 * walk covers only the siblings from `first` up to the exclusive `rangeEnd`
 * boundary.  Index `0` is still the section root, so root-level slots resolve
 * against that parent within the same bounds.
 *
 * Iterative by design - templates nest arbitrarily deep and recursion would put
 * that on the call stack.
 *
 * Cost is linear in the subtree this section owns.  Structurally nested blocks
 * are not free, though: each `<if>` / `<for>` hydrates its own body, and the
 * enclosing walk has already stepped over that body to reach its sibling, so a
 * chain of N nested blocks does O(N²) marker steps overall.  N is nesting
 * depth, not node count - at a realistic depth of 8 that is ~80 steps for the
 * whole chain - so `test-hydration-nested` in the hydration benchmark tracks it
 * rather than paying for a cross-section marker-pairing cache.
 */
export function buildSSRIndex(
  shape: readonly number[],
  ssrRoot: Node,
  needMarkers: boolean,
  first: ChildNode | null = ssrRoot.firstChild,
  rangeEnd: Node | null = null,
): SSRIndex {
  const elements: Array<Node | undefined> = [ssrRoot];
  const conds: Comment[] = [];
  const repeats: Comment[] = [];
  const raws: Array<[Comment, Comment]> = [];
  // Only the root frame is bounded.  A block range shares its parent with the
  // following item or block, so an unbounded frame would run past the range's
  // closing marker once the template ran out and adopt a following sibling
  // block's marker as its own.
  const stack: Array<{ remaining: number; s: ChildNode | null; end: Node | null }> = [
    { remaining: shape[0] >> 1, s: first, end: rangeEnd },
  ];
  let index = 0;

  while (stack.length > 0) {
    const frame = stack[stack.length - 1];
    const frameEnd = frame.end;

    // Advance the SSR cursor to the next element, recording block markers in
    // document order and stepping over their ranges.
    let s = frame.s;
    while (s && s !== frameEnd) {
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
        if (isRawStartMarker(data)) {
          const start = s as Comment;
          let end: ChildNode | null = start.nextSibling;
          let depth = 1;
          const endMarker = `/${data}`;
          while (end && depth > 0) {
            if (end.nodeType === 8 /* COMMENT_NODE */) {
              const marker = (end as Comment).data;
              if (marker === data) depth++;
              else if (marker === endMarker) depth--;
            }
            if (depth > 0) end = end.nextSibling;
          }
          if (end) {
            raws.push([start, end as Comment]);
            s = end.nextSibling;
          } else {
            s = null;
          }
          continue;
        }
      } else if (type === 1 /* ELEMENT_NODE */) {
        // A compiler-emitted style fallback is server-only: `meta.h` never
        // contains it, so counting it would pair every following template
        // element with its predecessor's node. `findByOrdinal` skips it for
        // the same reason.
        if (!isComponentStyleResourceMarker(s as Element)) break;
      }
      s = s.nextSibling;
    }
    if (s === frameEnd) s = null;

    if (frame.remaining === 0) {
      stack.pop();
      continue;
    }

    // The counter follows the template, never the server output, so a run of
    // missing SSR elements leaves holes rather than shifting every index after
    // it onto the wrong node.
    index++;
    if (s) {
      elements[index] = s;
      frame.s = s.nextSibling;
    }
    frame.remaining--;

    // Descend where the static topology owns children. An empty element can
    // still hold block markers -
    // `<ul><for ...></ul>` compiles to an empty `<ul>` - so it is worth walking
    // too, but only when this section actually has blocks to place.  A custom
    // element is the exception either way: with no template children it
    // contributes no projected content, and whatever the server rendered inside
    // belongs to that component, not to this one.
    const childShape = shape[index];
    const childCount = childShape >> 1;
    if (childCount > 0 || (needMarkers && (childShape & 1) && s?.firstChild)) {
      stack.push({ remaining: childCount, s: s ? s.firstChild : null, end: null });
    }
  }

  return { elements, conds, repeats, raws };
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
 * block range or compiler-emitted style fallback. Nested blocks of the same
 * type are handled via depth tracking. Returns the child at the given
 * `ordinal`, or null.
 *
 * Used by static slot fallback to keep SSR nodes aligned with the template.
 * Element bindings resolve directly from the stable pre-order element table
 * built by `buildSSRIndex`. Text bindings cannot use that table because SSR
 * strips formatting-whitespace text nodes that remain in `meta.h`; their
 * fallback boundaries are therefore located by node type and ordinal.
 *
 * **Requires closing markers to still be in the DOM** - caller must retain
 * `<!--/wc-->`, `<!--/wr-->`, and paired raw-range end markers until all
 * resolution is done.
 *
 * A block range passes its first node and exclusive `end` boundary so ordinals
 * count only that range rather than every child of the shared parent.
 */
export function findByOrdinal(
  parent: Node,
  nodeType: number,
  ordinal: number,
  first: ChildNode | null = parent.firstChild,
  end: Node | null = null,
): Node | null {
  let count = 0;
  let child = first;
  while (child && child !== end) {
    // Detect a structural block opening marker and skip the entire range.
    if (child.nodeType === 8 /* COMMENT_NODE */) {
      const data = (child as Comment).data;
      if (
        data === MARKER_COND_START
        || data === MARKER_REPEAT_START
        || isRawStartMarker(data)
      ) {
        child = skipBlockRange(child as Comment, data);
        continue;
      }
    }
    const isStyleResource = child.nodeType === 1 &&
      isComponentStyleResourceMarker(child as Element);
    if (child.nodeType === nodeType && !isStyleResource) {
      if (count === ordinal) return child;
      count++;
    }
    child = child.nextSibling;
  }
  return null;
}
