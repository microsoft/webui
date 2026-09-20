// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { Router } from '@microsoft/webui-router';
import './component-definitions';
import { routerConfig } from './router-config';

window.addEventListener('webui:hydration-complete', () => {
  Router.start(routerConfig);
});
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  Router.start(routerConfig);
}

(window as unknown as Record<string, unknown>).__testRouter = Router;
