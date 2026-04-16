// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable, attr } from '@microsoft/webui-framework';
import { Router } from '@microsoft/webui-router';

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

// ── Start router after hydration ─────────────────────────────────

window.addEventListener('webui:hydration-complete', () => {
  Router.start({
    loaders: {
      'page-alpha': () => Promise.resolve(),
      'page-beta': () => Promise.resolve(),
      'page-detail': () => Promise.resolve(),
      'page-compose': () => Promise.resolve(),
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
    },
  });
}

// Expose Router for E2E tests
(window as unknown as Record<string, unknown>).__testRouter = Router;
