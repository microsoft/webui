// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import '@microsoft/webui-framework/template-cache.js';
import { WebUIRouter } from '@microsoft/webui-delivery/router.js';
import type { RouterConfig } from '@microsoft/webui-delivery/router.js';
import { ndjson } from '@microsoft/webui-delivery/resources.js';
import './component-definitions';
import { routerConfig } from './router-config';

const router = new WebUIRouter();
const start = () => router.start({
  ...routerConfig,
  ...(new URL(location.href).searchParams.has('ndjson') ? { transport: ndjson() } : {}),
  ...(window as unknown as { __testRouterConfig?: RouterConfig }).__testRouterConfig,
});
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) start();
else window.addEventListener('webui:hydration-complete', start, { once: true });

(window as unknown as { __testRouter: WebUIRouter }).__testRouter = router;
