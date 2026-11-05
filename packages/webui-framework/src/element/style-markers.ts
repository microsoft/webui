// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Whether an element is a compiler-owned Link/Style resource marker. */
export function isComponentStyleMarker(element: Element): boolean {
  if (typeof element.getAttribute !== 'function') return false;
  if (element.getAttribute('data-webui-resource') === null) return false;
  const strategy = element.getAttribute('data-webui-strategy');
  return (element.localName === 'link' && strategy === 'link') ||
    (element.localName === 'style' && (strategy === 'style' || strategy === 'module'));
}

/** Whether an element is any compiler-owned style resource marker. */
export function isComponentStyleResourceMarker(element: Element): boolean {
  if (isComponentStyleMarker(element)) return true;
  return element.localName === 'script' &&
    element.getAttribute('type') === 'importmap' &&
    element.getAttribute('data-webui-resource') !== null;
}
