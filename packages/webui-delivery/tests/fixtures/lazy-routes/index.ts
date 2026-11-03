// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import '@microsoft/webui-framework/styles.js';
import '@microsoft/webui-framework/template-cache.js';
import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import { WebUIRouter } from '@microsoft/webui-delivery/router.js';

const router = new WebUIRouter();
Object.assign(window, { lazyRouteRouter: router });
startRuntime();
router.start({
  loaders: {
    'preview-page': () => import('../../../../webui-framework/tests/fixtures/lazy-pages/src/preview-page.ts'),
    'unrelated-page': () => import('../../../../webui-framework/tests/fixtures/lazy-pages/src/unrelated-page.ts'),
  },
});
