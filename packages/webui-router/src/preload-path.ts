// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { stripBaseFromPathname } from './navigation-path.js';

/** Resolve one internal mouse-link intent to its request path. */
export function pointerPreloadPath(
  event: PointerEvent,
  basePath: string,
  excludePaths: readonly string[],
): string | null {
  if (event.pointerType !== 'mouse') return null;

  const path = event.composedPath();
  let anchor: HTMLAnchorElement | undefined;
  for (let i = 0; i < path.length; i++) {
    if ((path[i] as Element)?.tagName === 'A') {
      anchor = path[i] as HTMLAnchorElement;
      break;
    }
  }
  if (!anchor) return null;

  const href = anchor.getAttribute('href');
  if (!href || href.startsWith('#') || anchor.origin !== location.origin) {
    return null;
  }
  for (let i = 0; i < excludePaths.length; i++) {
    if (anchor.pathname.startsWith(excludePaths[i])) return null;
  }

  const stripped = stripBaseFromPathname(anchor.pathname, basePath);
  return (stripped + anchor.search) || '/';
}
