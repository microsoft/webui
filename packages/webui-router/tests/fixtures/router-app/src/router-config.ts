// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { RouterConfig } from '@microsoft/webui-router';

export const routerConfig: RouterConfig = {
  loaders: {
    'page-alpha': () => Promise.resolve(),
    'page-beta': () => Promise.resolve(),
    'route-dashboard': () => Promise.resolve(),
    'page-detail': () => Promise.resolve(),
    'page-compose': () => Promise.resolve(),
    'page-keepalive': () => Promise.resolve(),
    'page-loader': () => Promise.resolve(),
    'page-slow': () => Promise.resolve(),
    'page-failing': () => Promise.resolve(),
    'loading-skeleton': () => Promise.resolve(),
    'error-display': () => Promise.resolve(),
  },
};
