// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { RouteAdmission } from './types.js';

export function isNavigationExcluded(pathname: string, excludePaths: readonly string[]): boolean {
  for (let i = 0; i < excludePaths.length; i++) {
    if (pathname.startsWith(excludePaths[i])) return true;
  }
  return false;
}

export function admitsNavigation(
  url: URL,
  excludePaths: readonly string[],
  shouldIntercept?: RouteAdmission,
): boolean {
  return !isNavigationExcluded(url.pathname, excludePaths) && (!shouldIntercept || shouldIntercept(url));
}
