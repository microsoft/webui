// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export const BOUNDARY_SCRIPT_ATTR = 'data-webui-boundary';
export const MAX_ELEMENTS_PER_BOUNDARY = 10_000;
export const MAX_MARKER_SCAN_NODES = 50_000;
export const MAX_BOUNDARY_SCRIPT_SCAN = 8;

const BOUNDARY_START_PREFIX = 'wb:';
const BOUNDARY_END_PREFIX = '/wb:';

/** The DOM span one committed boundary activates. */
export interface HydrationRange {
  readonly start: Comment | null;
  readonly end: Comment | null;
}

export type RangeResolution =
  | { readonly ok: true; readonly range: HydrationRange }
  | { readonly ok: false; readonly reason: string; readonly truncated: false }
  | {
      readonly ok: false;
      readonly reason: string;
      readonly truncated: true;
      readonly start: Comment | null;
    };

const MARKERLESS_RANGE: HydrationRange = { start: null, end: null };

/** Resolve the in-order marker range for one checkpoint. */
export function resolveBoundaryRange(
  scriptEl: Element,
  sequence: number,
  terminal: boolean,
): RangeResolution {
  const end = findEndMarker(scriptEl, sequence);
  if (end) {
    if (terminal) {
      return {
        ok: false,
        reason: `terminal boundary ${sequence} must be markerless`,
        truncated: false,
      };
    }
    const start = findStartMarkerBefore(end, sequence);
    if (!start) {
      return {
        ok: false,
        reason: `missing start marker for boundary ${sequence}`,
        truncated: false,
      };
    }
    return { ok: true, range: { start, end } };
  }
  if (terminal) return { ok: true, range: MARKERLESS_RANGE };
  return {
    ok: false,
    reason: `missing end marker for boundary ${sequence}`,
    truncated: true,
    start: findStartMarkerBefore(scriptEl, sequence),
  };
}

export function findBoundaryScript(sentinel: Element): Element | null {
  let node: Element | null = sentinel.previousElementSibling;
  for (
    let hops = 0;
    node && hops < MAX_BOUNDARY_SCRIPT_SCAN;
    hops++
  ) {
    if (
      node.tagName === 'SCRIPT' &&
      node.hasAttribute(BOUNDARY_SCRIPT_ATTR)
    ) {
      return node;
    }
    node = node.previousElementSibling;
  }
  return null;
}

function findEndMarker(
  scriptEl: Element,
  sequence: number,
): Comment | null {
  const marker = previousComment(scriptEl);
  return marker?.data === `${BOUNDARY_END_PREFIX}${sequence}`
    ? marker
    : null;
}

function findStartMarkerBefore(
  nodeBefore: Node,
  sequence: number,
): Comment | null {
  return findCommentBefore(
    nodeBefore,
    `${BOUNDARY_START_PREFIX}${sequence}`,
  );
}

/** Find a rejected boundary's end marker without a valid parsed sequence. */
export function findEndMarkerByPrefix(
  scriptEl: Element,
): Comment | null {
  const marker = previousComment(scriptEl);
  return marker?.data.startsWith(BOUNDARY_END_PREFIX) ? marker : null;
}

/** Find the start marker paired with a structurally discovered end marker. */
export function findStartMarkerByPrefix(
  endMarker: Comment,
): Comment | null {
  return findCommentBefore(
    endMarker,
    `${BOUNDARY_START_PREFIX}${endMarker.data.slice(
      BOUNDARY_END_PREFIX.length,
    )}`,
  );
}

function previousComment(node: Node): Comment | null {
  const previous = node.previousSibling;
  return previous?.nodeType === 8 /* COMMENT_NODE */
    ? (previous as Comment)
    : null;
}

function findCommentBefore(nodeBefore: Node, target: string): Comment | null {
  let node: Node | null = nodeBefore.previousSibling;
  for (let hops = 0; node && hops < MAX_MARKER_SCAN_NODES; hops++) {
    if (
      node.nodeType === 8 /* COMMENT_NODE */ &&
      (node as Comment).data === target
    ) {
      return node as Comment;
    }
    node = node.previousSibling;
  }
  return null;
}

/** Remove a node without letting a detached or hostile node throw. */
export function safeRemove(node: Node | null | undefined): void {
  try {
    (node as ChildNode | null | undefined)?.remove?.();
  } catch {
    // Already detached or non-removable.
  }
}

/** Remove an attribute without letting an exotic element throw. */
export function safeRemoveAttribute(el: Element, name: string): void {
  try {
    el.removeAttribute?.(name);
  } catch {
    // Non-removable attribute host.
  }
}

/** Remove generated payload, sentinel, extension scripts, and markers. */
export function removeBoundaryScaffolding(
  sentinel: Element,
  scriptEl: Element | null,
  startMarker: Comment | null,
  endMarker: Comment | null,
): void {
  if (!scriptEl) {
    safeRemove(sentinel);
    safeRemove(endMarker);
    safeRemove(startMarker);
    return;
  }

  let node: Element | null = sentinel.previousElementSibling;
  safeRemove(sentinel);
  for (
    let hops = 0;
    node && hops <= MAX_BOUNDARY_SCRIPT_SCAN;
    hops++
  ) {
    const previous: Element | null = node.previousElementSibling;
    const wasBoundaryScript = node === scriptEl;
    safeRemove(node);
    if (wasBoundaryScript) break;
    node = previous;
  }
  safeRemove(endMarker);
  safeRemove(startMarker);
}

/** First node in an element's boundary-order subtree. */
export function firstNodeWithin(root: Element): Node | null {
  return root.shadowRoot?.firstChild ?? root.firstChild;
}

/** Continue boundary-order traversal without leaving `root`. */
export function nextWithinRoot(node: Node, root: Node): Node | null {
  if (node.nodeType === 1 /* ELEMENT_NODE */) {
    const shadowRoot = (node as Element).shadowRoot;
    if (shadowRoot?.firstChild) return shadowRoot.firstChild;
  }
  if (node.firstChild) return node.firstChild;
  return nextAfterSubtreeWithin(node, root);
}

/** Return the node after `node`'s subtree without leaving `root`. */
export function nextAfterSubtreeWithin(
  node: Node,
  root: Node,
): Node | null {
  let current: Node | null = node;
  while (current && current !== root) {
    if (current.nodeType === 11 /* DOCUMENT_FRAGMENT_NODE */) {
      const host: Element | null = (current as ShadowRoot).host ?? null;
      if (host) {
        if (host.firstChild) return host.firstChild;
        current = host;
        continue;
      }
    }
    if (current.nextSibling) return current.nextSibling;
    current = current.parentNode;
  }
  return null;
}
