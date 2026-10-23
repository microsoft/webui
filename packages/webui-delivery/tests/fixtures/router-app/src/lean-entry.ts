// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import '@microsoft/webui-framework/template-cache.js';
import { WebUIRouter } from '@microsoft/webui-delivery/router.js';
import './component-definitions';
import { routerConfig } from './router-config';

const router = new WebUIRouter();
const start = () => router.start(routerConfig);
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) start();
else window.addEventListener('webui:hydration-complete', start, { once: true });

(window as unknown as { __testRouter: WebUIRouter }).__testRouter = router;
