// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { Router } from '@microsoft/webui-router';
import { ErrorDisplay } from './error-display/error-display';
import { LoadingSkeleton } from './loading-skeleton/loading-skeleton';
import { PageAlpha } from './page-alpha/page-alpha';
import { PageBeta } from './page-beta/page-beta';
import { PageCompose } from './page-compose/page-compose';
import { PageDetail } from './page-detail/page-detail';
import { PageFailing } from './page-failing/page-failing';
import { PageKeepAlive } from './page-keepalive/page-keepalive';
import { PageLoader } from './page-loader/page-loader';
import { PageSlow } from './page-slow/page-slow';
import { RouteShell } from './route-shell/route-shell';

// ── Shell component ──────────────────────────────────────────────

RouteShell.define('route-shell');

// ── Page components ──────────────────────────────────────────────

PageAlpha.define('page-alpha');

PageBeta.define('page-beta');

PageDetail.define('page-detail');

PageCompose.define('page-compose');

PageKeepAlive.define('page-keepalive');

// ── Loader test component ────────────────────────────────────────
// Has a static loader() that provides client-side state.

PageLoader.define('page-loader');

// ── Pending UI test: skeleton component ──────────────────────────

LoadingSkeleton.define('loading-skeleton');

// ── Pending UI test: slow-loading page component ─────────────────

PageSlow.define('page-slow');

ErrorDisplay.define('error-display');

// ── Error boundary test: failing page component ──────────────────

PageFailing.define('page-failing');

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
      'page-slow': () => Promise.resolve(),
      'page-failing': () => Promise.resolve(),
      'loading-skeleton': () => Promise.resolve(),
      'error-display': () => Promise.resolve(),
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
      'page-slow': () => Promise.resolve(),
      'page-failing': () => Promise.resolve(),
      'loading-skeleton': () => Promise.resolve(),
      'error-display': () => Promise.resolve(),
    },
  });
}

// Expose Router for E2E tests
(window as unknown as Record<string, unknown>).__testRouter = Router;
