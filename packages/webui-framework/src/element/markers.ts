// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Build the paired marker data for a client-created raw binding. */
export function rawMarker(index: number, closing = false): string {
  return `${closing ? '/w' : 'w'}${index}`;
}

/** Recognize a compiler raw-range opening label without parsing user markup. */
export function isRawStartMarker(data: string): boolean {
  if (data.length < 2 || data.charCodeAt(0) !== 119 /* w */) return false;
  for (let i = 1; i < data.length; i++) {
    const code = data.charCodeAt(i);
    if (code < 48 /* 0 */ || code > 57 /* 9 */) return false;
  }
  return true;
}

/**
 * Number cached/client-created template elements in compiler pre-order.
 * SSR sections use the shared hydrator; this walk never sees server ranges.
 */
export function collectTemplateElements(root: Node): Array<Node | undefined> {
  const elements: Array<Node | undefined> = [root];
  const stack: Array<ChildNode | null> = [root.firstChild];
  while (stack.length > 0) {
    let child = stack.pop() ?? null;
    while (child) {
      if (child.nodeType === 1 /* ELEMENT_NODE */) {
        elements.push(child);
        if (child.firstChild) {
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
