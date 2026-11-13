// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { stripBaseFromPathname } from './navigation-path.js';
import { isNavigationExcluded } from './navigation-admission.js';
import type { RouteAdmission } from './types.js';

/** Resolve one internal mouse-link intent to its request path. */
export function pointerPreloadPath(
  event: PointerEvent,
  basePath: string,
  excludePaths: readonly string[],
  shouldIntercept?: RouteAdmission,
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
  if (isNavigationExcluded(anchor.pathname, excludePaths)) return null;
  if (shouldIntercept && !shouldIntercept(new URL(anchor.href))) return null;

  const stripped = stripBaseFromPathname(anchor.pathname, basePath);
  return (stripped + anchor.search) || '/';
}
