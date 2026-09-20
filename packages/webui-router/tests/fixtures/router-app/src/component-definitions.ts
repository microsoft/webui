// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

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
import { RouteDashboard } from './route-dashboard/route-dashboard';
import { RouteShell } from './route-shell/route-shell';

RouteShell.define('route-shell');
PageAlpha.define('page-alpha');
PageBeta.define('page-beta');
RouteDashboard.define('route-dashboard');
PageDetail.define('page-detail');
PageCompose.define('page-compose');
PageKeepAlive.define('page-keepalive');
PageLoader.define('page-loader');
LoadingSkeleton.define('loading-skeleton');
PageSlow.define('page-slow');
ErrorDisplay.define('error-display');
PageFailing.define('page-failing');
