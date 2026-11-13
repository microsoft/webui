// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIRouter, routePreload, navigationCache } from '@microsoft/webui-delivery/router.js';
import type { RouterConfig } from '@microsoft/webui-delivery/router.js';
import './component-definitions';

const globals = window as unknown as {
  __testRouterConfig?: RouterConfig;
  __testRouter: WebUIRouter;
  __startRouter: () => void;
};
const config = globals.__testRouterConfig ?? {};
const options = {
  shouldIntercept: config.shouldIntercept,
  responsePolicy: config.responsePolicy,
  excludePaths: config.excludePaths,
};
const preload = routePreload(options);
const router = new WebUIRouter();
globals.__testRouter = router;
globals.__startRouter = () => router.start({
  ...config, preload, cache: navigationCache(),
});
if (new URL(location.href).searchParams.has('prepare')) {
  preload.prepare();
} else if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  globals.__startRouter();
} else {
  window.addEventListener('webui:hydration-complete', globals.__startRouter, { once: true });
}
