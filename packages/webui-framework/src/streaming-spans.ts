// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  MAX_MARKER_SCAN_NODES,
  safeRemove,
  safeRemoveAttribute,
  SPAN_START_PREFIX,
} from './streaming-dom.js';
import type { HydrationRange } from './streaming-dom.js';
import { STREAMING_SPAN_HOST_ATTR } from './streaming-mode.js';

/** Maximum unfinished component hosts retained by one response. */
export const MAX_OPEN_SPANS = 128;
/** Maximum runtime component ancestry crossed by one early boundary. */
export const MAX_SPAN_NESTING = 32;

interface OpenSpan {
  readonly host: Element;
  readonly start: Comment;
  readonly parentId: number | undefined;
  openChildren: number;
}

const openSpans = new Map<number, OpenSpan>();
// Reused by the single-record pump while it validates one ancestor chain.
const hostScratch: Element[] = [];
const idScratch: number[] = [];
let nextExpectedSpanInstanceId = 0;

/**
 * Register the unfinished component ancestry enclosing an early boundary.
 *
 * Span IDs are allocated where `<!--ws:S-->` starts, outer-first and gapless.
 * Walking from the boundary render root discovers them inner-first, so the two
 * module-level scratch arrays are replayed backwards without per-record maps.
 */
export function registerEnclosingSpans(
  renderRoot: Node,
  enclosingSpanInstanceId: number,
): string | null {
  hostScratch.length = 0;
  idScratch.length = 0;
  let current: Node | null = renderRoot;
  let hops = 0;
  let firstSpan = true;

  while (current && hops < MAX_MARKER_SCAN_NODES) {
    hops++;
    const element = elementForNode(current);
    if (
      element &&
      typeof element.hasAttribute === 'function' &&
      element.hasAttribute(STREAMING_SPAN_HOST_ATTR)
    ) {
      if (hostScratch.length >= MAX_SPAN_NESTING) {
        clearScratch();
        return `runtime component span nesting exceeds ${MAX_SPAN_NESTING}`;
      }
      const id = parseInstanceId(
        element.getAttribute(STREAMING_SPAN_HOST_ATTR),
      );
      if (id === null) {
        clearScratch();
        return `invalid ${STREAMING_SPAN_HOST_ATTR} value`;
      }
      if (firstSpan && id !== enclosingSpanInstanceId) {
        clearScratch();
        return `boundary declares enclosing span ${enclosingSpanInstanceId}, but its nearest spanning ancestor is ${id}`;
      }
      firstSpan = false;
      hostScratch.push(element);
      idScratch.push(id);
    }
    current = parentAcrossRenderRoot(current, element);
  }

  if (firstSpan) {
    clearScratch();
    return `boundary declares enclosing span ${enclosingSpanInstanceId}, but no spanning ancestor was found`;
  }
  if (current) {
    clearScratch();
    return `spanning ancestor walk exceeds ${MAX_MARKER_SCAN_NODES} nodes`;
  }

  for (let i = hostScratch.length - 1; i >= 0; i--) {
    const error = registerSpan(
      idScratch[i],
      hostScratch[i],
      i + 1 < idScratch.length ? idScratch[i + 1] : undefined,
    );
    if (error) {
      clearScratch();
      return error;
    }
  }
  clearScratch();
  return null;
}

function registerSpan(
  id: number,
  host: Element,
  parentId: number | undefined,
): string | null {
  const existing = openSpans.get(id);
  if (existing) {
    return existing.host === host && existing.parentId === parentId
      ? null
      : `span instance ${id} is already open for another ancestry`;
  }
  if (id !== nextExpectedSpanInstanceId) {
    return `expected span instance ${nextExpectedSpanInstanceId}, received ${id}`;
  }
  if (openSpans.size >= MAX_OPEN_SPANS) {
    return `open component span count exceeds ${MAX_OPEN_SPANS}`;
  }
  const marker = host.previousSibling;
  if (
    marker?.nodeType !== 8 /* COMMENT_NODE */ ||
    (marker as Comment).data !== `${SPAN_START_PREFIX}${id}`
  ) {
    return `missing root-local start marker for span ${id}`;
  }
  const parent = parentId === undefined ? undefined : openSpans.get(parentId);
  if (parentId !== undefined && !parent) {
    return `parent span ${parentId} is not open`;
  }

  openSpans.set(id, {
    host,
    start: marker as Comment,
    parentId,
    openChildren: 0,
  });
  if (parent) parent.openChildren++;
  nextExpectedSpanInstanceId++;
  return null;
}

/**
 * Discover a previously unseen completion target from its concrete marker range.
 *
 * Zero-occurrence component spans have no checkpoint to register their
 * ancestry, so their completion must do the same bounded, root-local discovery.
 */
export function registerSpanCompletionTarget(
  id: number,
  range: HydrationRange,
): string | null {
  if (openSpans.has(id)) return null;
  if (!range.start || !range.end) {
    return `span ${id} completion is markerless`;
  }

  let node = range.start.nextSibling;
  let hops = 0;
  while (node && node !== range.end) {
    if (hops >= MAX_MARKER_SCAN_NODES) {
      return `span ${id} host lookup exceeds ${MAX_MARKER_SCAN_NODES} nodes`;
    }
    hops++;
    if (node.nodeType === 1 /* ELEMENT_NODE */) {
      const element = node as Element;
      if (
        typeof element.hasAttribute === 'function' &&
        element.hasAttribute(STREAMING_SPAN_HOST_ATTR)
      ) {
        const hostId = parseInstanceId(
          element.getAttribute(STREAMING_SPAN_HOST_ATTR),
        );
        if (hostId === null) {
          return `invalid ${STREAMING_SPAN_HOST_ATTR} value`;
        }
        if (hostId !== id) {
          return `span completion targets span ${id}, but its host declares span ${hostId}`;
        }
        return registerEnclosingSpans(element, id);
      }
    }
    node = node.nextSibling;
  }
  return `span completion targets span ${id}, but no spanning host was found inside its markers`;
}

/** Validate one span completion before mutating or hydrating its range. */
export function validateSpanCompletion(
  id: number,
  range: HydrationRange,
): string | null {
  const span = openSpans.get(id);
  if (!span) return `span completion targets span ${id}, which is not open`;
  if (span.openChildren !== 0) {
    return `span ${id} completed before its nested component spans`;
  }
  if (
    !range.start ||
    !range.end ||
    range.start !== span.start ||
    span.host.parentNode !== range.start.parentNode
  ) {
    return `span completion markers do not match the open span ${id}`;
  }

  let node = range.start.nextSibling;
  let hops = 0;
  while (node && node !== range.end && node !== span.host) {
    if (hops >= MAX_MARKER_SCAN_NODES) {
      return `span ${id} host lookup exceeds ${MAX_MARKER_SCAN_NODES} nodes`;
    }
    hops++;
    node = node.nextSibling;
  }
  return node === span.host
    ? null
    : `span ${id} host is outside its completion markers`;
}

/** Release one successfully completed span and its ancestry accounting. */
export function completeSpan(id: number): void {
  const span = openSpans.get(id);
  if (!span) return;
  openSpans.delete(id);
  safeRemoveAttribute(span.host, STREAMING_SPAN_HOST_ATTR);
  if (span.parentId === undefined) return;
  const parent = openSpans.get(span.parentId);
  if (parent && parent.openChildren > 0) parent.openChildren--;
}

/** Fail-closed release of every retained span host and opening marker. */
export function abandonOpenSpans(): void {
  for (const span of openSpans.values()) {
    safeRemoveAttribute(span.host, STREAMING_SPAN_HOST_ATTR);
    safeRemove(span.start);
  }
  openSpans.clear();
  clearScratch();
  nextExpectedSpanInstanceId = 0;
}

function elementForNode(node: Node): Element | null {
  if (node.nodeType === 1 /* ELEMENT_NODE */) return node as Element;
  if (node.nodeType === 11 /* DOCUMENT_FRAGMENT_NODE */) {
    return (node as ShadowRoot).host ?? null;
  }
  return null;
}

function parentAcrossRenderRoot(
  node: Node,
  element: Element | null,
): Node | null {
  let current = node;
  let currentElement = element;
  if (node.nodeType === 11 /* DOCUMENT_FRAGMENT_NODE */) {
    const host = (node as ShadowRoot).host;
    if (!host) return null;
    current = host;
    currentElement = host;
  }
  if (currentElement?.assignedSlot) return currentElement.assignedSlot;
  const parent = current.parentNode;
  if (parent?.nodeType === 11 /* DOCUMENT_FRAGMENT_NODE */) {
    return (parent as ShadowRoot).host ?? null;
  }
  return parent;
}

function parseInstanceId(raw: string | null): number | null {
  if (raw === null || raw.length === 0) return null;
  let value = 0;
  for (let i = 0; i < raw.length; i++) {
    const code = raw.charCodeAt(i) - 48;
    if (code < 0 || code > 9) return null;
    value = value * 10 + code;
    if (!Number.isSafeInteger(value)) return null;
  }
  return String(value) === raw ? value : null;
}

function clearScratch(): void {
  hostScratch.length = 0;
  idScratch.length = 0;
}

export function openSpanCountForTests(): number {
  return openSpans.size;
}

/** Whether terminal validation still has unfinished component spans. */
export function hasOpenSpans(): boolean {
  return openSpans.size !== 0;
}
