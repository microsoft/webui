// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { Router } from '@microsoft/webui-router';
import './component-definitions';
import { routerConfig } from './router-config';

const config = new URL(location.href).searchParams.get('cache') === 'zero'
  ? { ...routerConfig, cache: {} }
  : routerConfig;

window.addEventListener('webui:hydration-complete', () => {
  Router.start(config);
});
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  Router.start(config);
}

(window as unknown as Record<string, unknown>).__testRouter = Router;
