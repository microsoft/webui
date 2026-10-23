// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIRouter, routeBoundaries, viewTransitions } from '@microsoft/webui-delivery/router.js';
import { ndjson } from '@microsoft/webui-delivery/resources.js';
import './component-definitions';
import { routerConfig } from './router-config';

const router = new WebUIRouter();
const start = () => router.start({
  ...routerConfig, transport: ndjson(), boundaries: routeBoundaries, transitions: viewTransitions,
});
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) start();
else window.addEventListener('webui:hydration-complete', start, { once: true });

(window as unknown as Record<string, unknown>).__testRouter = router;
