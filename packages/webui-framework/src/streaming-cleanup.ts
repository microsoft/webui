// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  firstNodeWithin,
  MAX_MARKER_SCAN_NODES,
  nextWithinRoot,
  safeRemoveAttribute,
  streamingErrorMessage,
} from './streaming-dom.js';
import { STREAMED_HOST_ATTR } from './streaming-mode.js';

const STREAMING_BOUNDARY_ABANDON = Symbol.for(
  'microsoft.webui.boundaryAbandon',
);

type BoundaryAbandonable = Element & {
  [STREAMING_BOUNDARY_ABANDON]?: () => void;
};

/** Clear both coordinator and element-owned streaming state from one root. */
export function abandonDeferredElement(el: Element): void {
  try {
    if (!el.hasAttribute(STREAMED_HOST_ATTR)) return;
  } catch {
    safeRemoveAttribute(el, STREAMED_HOST_ATTR);
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
    safeRemoveAttribute(el, STREAMED_HOST_ATTR);
  }
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
}
