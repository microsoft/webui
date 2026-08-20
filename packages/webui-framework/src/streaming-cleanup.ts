// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  BOUNDARY_END_PREFIX,
  BOUNDARY_SCRIPT_ATTR,
  BOUNDARY_START_PREFIX,
  firstNodeWithin,
  MAX_MARKER_SCAN_NODES,
  nextAfterSubtreeWithin,
  nextWithinRoot,
  safeRemoveAttribute,
  safeRemove,
  SPAN_END_PREFIX,
  SPAN_START_PREFIX,
  streamingErrorMessage,
} from './streaming-dom.js';
import {
  STREAMED_HOST_ATTR,
  STREAMING_ENCLOSING_SPAN_ATTR,
  STREAMING_SPAN_HOST_ATTR,
} from './streaming-mode.js';

const STREAMING_BOUNDARY_ABANDON = Symbol.for(
  'microsoft.webui.boundaryAbandon',
);

type BoundaryAbandonable = Element & {
  [STREAMING_BOUNDARY_ABANDON]?: () => void;
};

/** Clear both coordinator and element-owned streaming state from one root. */
export function abandonDeferredElement(el: Element): void {
  const marked = hasStreamingAttribute(el);
  try {
    if (!marked) return;
  } catch {
    removeStreamingAttributes(el);
    return;
  }
  try {
    const hook = (el as BoundaryAbandonable)[
      STREAMING_BOUNDARY_ABANDON
    ];
    if (typeof hook === 'function') hook.call(el);
  } catch (error) {
    console.error(
      `[WebUI] streaming: abandon failed for <${
        el.tagName?.toLowerCase?.() ?? '?'
      }>: ${streamingErrorMessage(error)}`,
    );
  } finally {
    removeStreamingAttributes(el);
  }
}

function hasStreamingAttribute(el: Element): boolean {
  return el.hasAttribute(STREAMED_HOST_ATTR) ||
    el.hasAttribute(STREAMING_SPAN_HOST_ATTR) ||
    el.hasAttribute(STREAMING_ENCLOSING_SPAN_ATTR);
}

function removeStreamingAttributes(el: Element): void {
  safeRemoveAttribute(el, STREAMED_HOST_ATTR);
  safeRemoveAttribute(el, STREAMING_SPAN_HOST_ATTR);
  safeRemoveAttribute(el, STREAMING_ENCLOSING_SPAN_ATTR);
}

/** Release a retained subtree after its undefined outer fails activation. */
export function abandonDeferredDescendants(root: Element): void {
  abandonDeferredNodes(firstNodeWithin(root), root, null);
}

/** Strip streamed state from one marker-delimited range. */
export function abandonDeferredRange(
  startMarker: Comment,
  endMarker: Node,
): void {
  const root = startMarker.parentNode;
  if (root) {
    abandonDeferredNodes(startMarker.nextSibling, root, endMarker);
  }
}

function abandonDeferredNodes(
  first: Node | null,
  root: Node,
  end: Node | null,
): void {
  let node = first;
  let visited = 0;
  while (node && node !== end && visited < MAX_MARKER_SCAN_NODES) {
    visited++;
    if (node.nodeType === 1 /* ELEMENT_NODE */) {
      abandonDeferredElement(node as Element);
    }
    node = nextWithinRoot(node, root);
  }
}

/** Bounded failure-only sweep for roots preceding a missing sentinel. */
export function abandonDeferredDocumentRoots(): void {
  if (
    typeof document === 'undefined' ||
    typeof document.getElementsByTagName !== 'function'
  ) {
    return;
  }

  const documentRoot = document.documentElement;
  if (documentRoot) {
    abandonStreamingNodes(documentRoot.firstChild, documentRoot);
    abandonDeferredElement(documentRoot);
    return;
  }

  const elements = document.getElementsByTagName('*');
  let visited = 0;
  for (
    let i = 0;
    i < elements.length && visited < MAX_MARKER_SCAN_NODES;
    i++
  ) {
    const el = elements[i];
    visited++;
    abandonDeferredElement(el);
    const shadowRoot = el.shadowRoot;
    let node: Node | null = shadowRoot?.firstChild ?? null;
    while (node && visited < MAX_MARKER_SCAN_NODES) {
      visited++;
      if (node.nodeType === 1 /* ELEMENT_NODE */) {
        abandonDeferredElement(node as Element);
      }
      node = nextWithinRoot(node, shadowRoot!);
    }
  }

  function abandonStreamingNodes(first: Node | null, root: Node): void {
    let node = first;
    let visited = 0;
    while (node && visited < MAX_MARKER_SCAN_NODES) {
      visited++;
      if (node.nodeType === 8 /* COMMENT_NODE */) {
        const next = nextWithinRoot(node, root);
        if (isStreamingMarker((node as Comment).data)) safeRemove(node);
        node = next;
        continue;
      }
      if (node.nodeType === 1 /* ELEMENT_NODE */) {
        const el = node as Element;
        const scaffold = el.tagName === 'WEBUI-HYDRATE' ||
          el.hasAttribute(BOUNDARY_SCRIPT_ATTR);
        const next = scaffold
          ? nextAfterSubtreeWithin(node, root)
          : nextWithinRoot(node, root);
        abandonDeferredElement(el);
        if (scaffold) safeRemove(el);
        node = next;
        continue;
      }
      node = nextWithinRoot(node, root);
    }
  }

  function isStreamingMarker(data: string): boolean {
    return data.startsWith(BOUNDARY_START_PREFIX) ||
      data.startsWith(BOUNDARY_END_PREFIX) ||
      data.startsWith(SPAN_START_PREFIX) ||
      data.startsWith(SPAN_END_PREFIX);
  }
}
