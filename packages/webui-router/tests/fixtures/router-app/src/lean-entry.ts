// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import '@microsoft/webui-framework/template-cache.js';
import { WebUIRouter } from '@microsoft/webui-router/core.js';
import './component-definitions';
import { routerConfig } from './router-config';

const router = new WebUIRouter();
window.addEventListener('webui:hydration-complete', () => {
  router.start(routerConfig);
});
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  router.start(routerConfig);
}

(window as unknown as { __testRouter: WebUIRouter }).__testRouter = router;
