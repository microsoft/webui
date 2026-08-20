// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export const BOUNDARY_SCRIPT_ATTR = 'data-webui-boundary';
export const MAX_ELEMENTS_PER_BOUNDARY = 10_000;
export const MAX_MARKER_SCAN_NODES = 50_000;
export const MAX_BOUNDARY_SCRIPT_SCAN = 8;

/** Normalize an unknown exception for cold streaming diagnostics. */
export function streamingErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Root-local runtime BoundaryInstanceId marker prefixes. */
export const BOUNDARY_START_PREFIX = 'wb:';
export const BOUNDARY_END_PREFIX = '/wb:';
/** Root-local runtime SpanInstanceId marker prefixes. */
export const SPAN_START_PREFIX = 'ws:';
export const SPAN_END_PREFIX = '/ws:';

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

/** Resolve the root-local marker range for one boundary occurrence. */
export function resolveBoundaryRange(
  scriptEl: Element,
  instanceId: number,
): RangeResolution {
  return resolveMarkerRange(
    scriptEl,
    instanceId,
    BOUNDARY_START_PREFIX,
    BOUNDARY_END_PREFIX,
    'boundary',
  );
}

/** Resolve the root-local marker range for one completed component span. */
export function resolveSpanRange(
  scriptEl: Element,
  instanceId: number,
): RangeResolution {
  return resolveMarkerRange(
    scriptEl,
    instanceId,
    SPAN_START_PREFIX,
    SPAN_END_PREFIX,
    'span',
  );
}

function resolveMarkerRange(
  scriptEl: Element,
  instanceId: number,
  startPrefix: string,
  endPrefix: string,
  kind: string,
): RangeResolution {
  const end = findEndMarker(scriptEl, instanceId, endPrefix);
  if (end) {
    const start = findStartMarkerBefore(end, instanceId, startPrefix);
    if (!start) {
      return {
        ok: false,
        reason: `missing start marker for ${kind} ${instanceId}`,
        truncated: false,
      };
    }
    return { ok: true, range: { start, end } };
  }
  return {
    ok: false,
    reason: `missing end marker for ${kind} ${instanceId}`,
    truncated: true,
    start: findStartMarkerBefore(scriptEl, instanceId, startPrefix),
  };
}

/** Require an update or terminal record to carry no range markers. */
export function resolveMarkerlessRecord(
  scriptEl: Element,
  kind: string,
): RangeResolution {
  const marker = previousComment(scriptEl);
  if (
    marker &&
    (marker.data.startsWith(BOUNDARY_END_PREFIX) ||
      marker.data.startsWith(SPAN_END_PREFIX))
  ) {
    return {
      ok: false,
      reason: `${kind} record must be markerless`,
      truncated: false,
    };
  }
  return { ok: true, range: MARKERLESS_RANGE };
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
  instanceId: number,
  prefix: string,
): Comment | null {
  const marker = previousComment(scriptEl);
  return marker?.data === `${prefix}${instanceId}`
    ? marker
    : null;
}

function findStartMarkerBefore(
  nodeBefore: Node,
  instanceId: number,
  prefix: string,
): Comment | null {
  return findCommentBefore(
    nodeBefore,
    `${prefix}${instanceId}`,
  );
}

/** Find a rejected record's end marker without a valid parsed target. */
export function findRangeEndMarkerByPrefix(
  scriptEl: Element,
): Comment | null {
  const marker = previousComment(scriptEl);
  return marker &&
      (marker.data.startsWith(BOUNDARY_END_PREFIX) ||
        marker.data.startsWith(SPAN_END_PREFIX))
    ? marker
    : null;
}

/** Find the start marker paired with a structurally discovered end marker. */
export function findRangeStartMarkerByPrefix(
  endMarker: Comment,
): Comment | null {
  const endPrefix = endMarker.data.startsWith(BOUNDARY_END_PREFIX)
    ? BOUNDARY_END_PREFIX
    : SPAN_END_PREFIX;
  const startPrefix = endPrefix === BOUNDARY_END_PREFIX
    ? BOUNDARY_START_PREFIX
    : SPAN_START_PREFIX;
  return findCommentBefore(
    endMarker,
    `${startPrefix}${endMarker.data.slice(
      endPrefix.length,
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
