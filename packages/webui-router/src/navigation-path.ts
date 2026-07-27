// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export interface NavigationTarget {
  requestPath: string;
}

export function stripBaseFromPathname(pathname: string, basePath = ''): string {
  if (basePath && pathname.startsWith(basePath)) {
    return pathname.slice(basePath.length) || '/';
  }
  return pathname;
}

export function buildNavigationTarget(url: URL, basePath = ''): NavigationTarget {
  const pathname = stripBaseFromPathname(url.pathname, basePath);
  return {
    requestPath: `${pathname}${url.search}`,
  };
}

export function prependBasePath(path: string, basePath = ''): string {
  return `${basePath}${path}`;
}
