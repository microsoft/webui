// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  MAX_MARKER_SCAN_NODES,
  safeRemove,
  safeRemoveAttribute,
  SPAN_START_PREFIX,
  STREAMING_SPAN_HOST_ATTR,
} from './streaming-dom.js';
import type { HydrationRange } from './streaming-dom.js';

/** Maximum unfinished component hosts retained by one response. */
export const MAX_OPEN_SPANS = 128;
/** Maximum runtime component ancestry crossed by one early boundary. */
export const MAX_SPAN_NESTING = 32;

const INVALID_SPAN_ID = -1;
const NOT_A_SPAN_HOST = -2;

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
 * Read one element's declared SpanInstanceId.
 *
 * `NOT_A_SPAN_HOST` when the element carries no marker at all and
 * `INVALID_SPAN_ID` when it carries one that is not a canonical base-10
 * integer, so both discovery walks share one attribute read and one parse.
 */
function spanIdOf(element: Element): number {
  if (typeof element.hasAttribute !== 'function') return NOT_A_SPAN_HOST;
  if (!element.hasAttribute(STREAMING_SPAN_HOST_ATTR)) return NOT_A_SPAN_HOST;
  const raw = element.getAttribute(STREAMING_SPAN_HOST_ATTR);
  if (raw === null || raw.length === 0) return INVALID_SPAN_ID;
  let value = 0;
  for (let i = 0; i < raw.length; i++) {
    const code = raw.charCodeAt(i) - 48;
    if (code < 0 || code > 9) return INVALID_SPAN_ID;
    value = value * 10 + code;
    if (!Number.isSafeInteger(value)) return INVALID_SPAN_ID;
  }
  return String(value) === raw ? value : INVALID_SPAN_ID;
}

function invalidSpanAttrReason(): string {
  return `invalid ${STREAMING_SPAN_HOST_ATTR} value`;
}

/**
 * The element one already-registered span will eventually activate.
 *
 * The coordinator hands this to the activation walk so an entitled early root
 * can skip that exact ancestor by identity — no attribute name, and no span
 * bookkeeping, ever reaches the always-shipped bundle.
 */
export function spanHostFor(id: number): Element | undefined {
  return openSpans.get(id)?.host;
}

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
    const element = current.nodeType === 1 /* ELEMENT_NODE */
      ? current as Element
      : null;
    const id = element ? spanIdOf(element) : NOT_A_SPAN_HOST;
    if (id !== NOT_A_SPAN_HOST) {
      if (hostScratch.length >= MAX_SPAN_NESTING) {
        clearScratch();
        return `runtime component span nesting exceeds ${MAX_SPAN_NESTING}`;
      }
      if (id === INVALID_SPAN_ID) {
        clearScratch();
        return invalidSpanAttrReason();
      }
      if (firstSpan && id !== enclosingSpanInstanceId) {
        clearScratch();
        return `boundary declares enclosing span ${enclosingSpanInstanceId}, but its nearest spanning ancestor is ${id}`;
      }
      firstSpan = false;
      hostScratch.push(element as Element);
      idScratch.push(id);
    }
    current = ascendRenderRoots(current);
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
 * Resolve and validate one span completion before it mutates or hydrates.
 *
 * One bounded, root-local sibling scan serves both jobs. A span opened by an
 * earlier checkpoint is already registered and only needs its recorded host and
 * marker confirmed; a zero-occurrence span has never been seen at all, so the
 * same scan discovers the host its ancestry is registered from.
 */
export function prepareSpanCompletion(
  id: number,
  range: HydrationRange,
): string | null {
  const start = range.start;
  const end = range.end;
  if (!start || !end) return `span ${id} completion is markerless`;

  let host: Element | undefined;
  let node: Node | null = start.nextSibling;
  let hops = 0;
  while (node && node !== end) {
    if (hops >= MAX_MARKER_SCAN_NODES) {
      return `span ${id} host lookup exceeds ${MAX_MARKER_SCAN_NODES} nodes`;
    }
    hops++;
    if (node.nodeType === 1 /* ELEMENT_NODE */) {
      const hostId = spanIdOf(node as Element);
      if (hostId === INVALID_SPAN_ID) return invalidSpanAttrReason();
      if (hostId !== NOT_A_SPAN_HOST) {
        if (hostId !== id) {
          return `span completion targets span ${id}, but its host declares span ${hostId}`;
        }
        host = node as Element;
        break;
      }
    }
    node = node.nextSibling;
  }
  if (!host) {
    return `span completion targets span ${id}, but no spanning host was found inside its markers`;
  }

  if (!openSpans.has(id)) {
    const error = registerEnclosingSpans(host, id);
    if (error) return error;
  }
  const span = openSpans.get(id);
  if (!span) return `span completion targets span ${id}, which is not open`;
  if (span.openChildren !== 0) {
    return `span ${id} completed before its nested component spans`;
  }
  return span.host === host && span.start === start &&
      host.parentNode === start.parentNode
    ? null
    : `span completion markers do not match the open span ${id}`;
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

/**
 * Step one level out of the current render root.
 *
 * A shadow root resolves to its host, a slotted element to its assigned slot,
 * and anything else to its parent node — so one function crosses every render
 * root boundary an ancestry walk can hit.
 */
function ascendRenderRoots(node: Node): Node | null {
  if (node.nodeType === 11 /* DOCUMENT_FRAGMENT_NODE */) {
    return (node as ShadowRoot).host ?? null;
  }
  if (node.nodeType === 1 /* ELEMENT_NODE */) {
    const slot = (node as Element).assignedSlot;
    if (slot) return slot;
  }
  return node.parentNode;
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
