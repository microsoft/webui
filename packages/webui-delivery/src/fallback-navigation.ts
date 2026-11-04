// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Same-document navigation for hosts where the Navigation API cannot intercept.
 *
 * WebKit only grants `NavigateEvent.canIntercept` to documents served from an
 * HTTP(S)-family origin. Desktop shells serve the application from a custom
 * scheme (`webui://app`), so `window.navigation` exists and dispatches events
 * while every event reports `canIntercept: false`. Without a fallback the
 * router resolves each route through a full document load, which forces the
 * web engine to retain a separate document per navigation.
 *
 * This module reproduces the interception contract using `history.pushState`
 * and `popstate`, which remain fully functional on custom schemes. Browsers
 * without the Navigation API at all take the same path.
 */

export interface FallbackNavigationContext {
  readonly excludePaths: readonly string[];
  /** Run a same-document navigation for `url`, honoring `signal`. */
  navigate(url: URL, signal: AbortSignal): Promise<void>;
}

/** The parts of a click the interception decision depends on. */
export interface ClickIntent {
  defaultPrevented: boolean;
  /** `MouseEvent.button`; only the primary button navigates. */
  button: number;
  /** True when meta, ctrl, shift, or alt is held. */
  modified: boolean;
}

/** The parts of an anchor the interception decision depends on. */
export interface LinkIntent {
  /** The raw `href` attribute, or null when absent. */
  href: string | null;
  /** The fully resolved absolute URL of the anchor. */
  resolved: string;
  download: boolean;
  target: string;
  rel: string | null;
}

/**
 * Report whether the Navigation API can intercept cross-document navigations.
 *
 * Interception requires both the API and an HTTP-family origin. WebKit refuses
 * to treat a navigation on a custom scheme as same-document, so every real
 * `NavigateEvent` there reports `canIntercept: false` even though the API is
 * present. The scheme is the only reliable signal available up front: probing
 * with `history.replaceState` is misleading because a same-document state
 * change *is* interceptable on a custom scheme while an actual navigation is
 * not.
 */
export function canInterceptNavigations(): boolean {
  if (!window.navigation) return false;
  const protocol = location.protocol;
  return protocol === 'https:' || protocol === 'http:';
}

function isExcluded(pathname: string, excludePaths: readonly string[]): boolean {
  for (let i = 0; i < excludePaths.length; i++) {
    if (pathname.startsWith(excludePaths[i])) return true;
  }
  return false;
}

/**
 * Decide whether a link click becomes a same-document navigation.
 *
 * Returns the destination URL when the router owns the click, or null when the
 * host should handle it natively. Pure, so the policy is testable without a DOM.
 */
export function resolveLinkNavigation(
  click: ClickIntent,
  link: LinkIntent,
  currentHref: string,
  excludePaths: readonly string[],
): URL | null {
  // Modified clicks and non-primary buttons are the user asking the host to
  // handle the link (new tab, download, context menu).
  if (click.defaultPrevented || click.button !== 0 || click.modified) return null;
  if (link.download) return null;
  if (link.target && link.target !== '_self') return null;
  if (link.rel && link.rel.split(/\s+/).includes('external')) return null;
  if (!link.href || link.href.startsWith('#')) return null;

  let url: URL;
  let current: URL;
  try {
    current = new URL(currentHref);
    url = new URL(link.resolved, currentHref);
  } catch {
    return null;
  }
  if (!sameNavigationOrigin(url, current)) return null;
  if (isExcluded(url.pathname, excludePaths)) return null;
  // Pure fragment changes stay with the browser so it can scroll.
  if (url.hash && url.pathname === current.pathname && url.search === current.search) return null;
  return url;
}

function sameNavigationOrigin(left: URL, right: URL): boolean {
  if (left.protocol === right.protocol && left.origin !== 'null' && right.origin !== 'null') {
    return left.origin === right.origin;
  }
  return left.protocol === right.protocol &&
    left.username === right.username &&
    left.password === right.password &&
    left.host === right.host;
}

/** Resolve the anchor that owns a click, crossing shadow boundaries. */
function anchorFor(event: MouseEvent): HTMLAnchorElement | null {
  const path = event.composedPath();
  for (let i = 0; i < path.length; i++) {
    const node = path[i];
    if (node instanceof HTMLAnchorElement) return node;
  }
  return null;
}

/**
 * Install click and popstate interception.
 *
 * Returns a cleanup function that removes every listener and aborts any
 * in-flight navigation.
 */
export function setupFallbackNavigation(context: FallbackNavigationContext): () => void {
  let controller: AbortController | null = null;

  const run = (url: URL): void => {
    controller?.abort();
    const active = new AbortController();
    controller = active;
    void context.navigate(url, active.signal).catch((error: unknown) => {
      if (error instanceof DOMException && error.name === 'AbortError') return;
      console.error('[Router] Navigation error:', error);
    });
  };

  const onClick = (event: MouseEvent): void => {
    const anchor = anchorFor(event);
    if (!anchor) return;
    const url = resolveLinkNavigation(
      {
        defaultPrevented: event.defaultPrevented,
        button: event.button,
        modified: event.metaKey || event.ctrlKey || event.shiftKey || event.altKey,
      },
      {
        href: anchor.getAttribute('href'),
        resolved: anchor.href,
        download: anchor.hasAttribute('download'),
        target: anchor.target,
        rel: anchor.getAttribute('rel'),
      },
      location.href,
      context.excludePaths,
    );
    if (!url) return;

    event.preventDefault();
    if (url.href === location.href) return;
    history.pushState(null, '', url.href);
    run(url);
  };

  const onPopState = (): void => {
    const url = new URL(location.href);
    if (isExcluded(url.pathname, context.excludePaths)) return;
    run(url);
  };

  document.addEventListener('click', onClick, { capture: true });
  window.addEventListener('popstate', onPopState);

  return () => {
    document.removeEventListener('click', onClick, { capture: true });
    window.removeEventListener('popstate', onPopState);
    controller?.abort();
    controller = null;
  };
}

/** Programmatic push navigation for the fallback path. */
export function fallbackPush(href: string): void {
  const url = new URL(href, location.href);
  if (url.href === location.href) return;
  history.pushState(null, '', url.href);
  window.dispatchEvent(new PopStateEvent('popstate'));
}
