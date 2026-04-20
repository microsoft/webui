// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable, attr } from '@microsoft/webui-framework';
import { Router } from '@microsoft/webui-router';
import type { RouteLoaderContext } from '@microsoft/webui-router';

// ── Shell component ──────────────────────────────────────────────

export class RouteShell extends WebUIElement {
  @observable isHome = false;
  @observable isAlpha = false;
  @observable isBeta = false;
  @observable isItem1 = false;
  @observable isItem2 = false;
}
RouteShell.define('route-shell');

// ── Page components ──────────────────────────────────────────────

export class PageAlpha extends WebUIElement {}
PageAlpha.define('page-alpha');

export class PageBeta extends WebUIElement {}
PageBeta.define('page-beta');

export class PageDetail extends WebUIElement {
  @observable itemId = '';
}
PageDetail.define('page-detail');

export class PageCompose extends WebUIElement {
  @attr action = '';
  @attr to = '';
  @attr subject = '';
}
PageCompose.define('page-compose');

// ── Keep-alive test component ────────────────────────────────────
// Has local state (clickCount) that should survive keep-alive reactivation.

export class PageKeepAlive extends WebUIElement {
  @observable clickCount = 0;

  onIncrement = (): void => {
    this.clickCount++;
  };
}
PageKeepAlive.define('page-keepalive');

// ── Loader test component ────────────────────────────────────────
// Has a static loader() that provides client-side state.

export class PageLoader extends WebUIElement {
  @observable source = '';
  @observable loaderMessage = '';

  static async loader(_ctx: RouteLoaderContext): Promise<Record<string, unknown>> {
    return {
      source: 'client-loader',
      loaderMessage: 'Data fetched by static loader',
    };
  }
}
PageLoader.define('page-loader');

// ── Start router after hydration ─────────────────────────────────

window.addEventListener('webui:hydration-complete', () => {
  Router.start({
    loaders: {
      'page-alpha': () => Promise.resolve(),
      'page-beta': () => Promise.resolve(),
      'page-detail': () => Promise.resolve(),
      'page-compose': () => Promise.resolve(),
      'page-keepalive': () => Promise.resolve(),
      'page-loader': () => Promise.resolve(),
    },
  });
});

// Fallback if hydration already completed
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  Router.start({
    loaders: {
      'page-alpha': () => Promise.resolve(),
      'page-beta': () => Promise.resolve(),
      'page-detail': () => Promise.resolve(),
      'page-compose': () => Promise.resolve(),
      'page-keepalive': () => Promise.resolve(),
      'page-loader': () => Promise.resolve(),
    },
  });
}

// Expose Router for E2E tests
(window as unknown as Record<string, unknown>).__testRouter = Router;
