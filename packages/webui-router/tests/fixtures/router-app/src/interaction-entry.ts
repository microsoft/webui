// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  installInteractionHydration,
  wakeInteractionHydration,
} from '@microsoft/webui-framework/interaction-hydration.js';
import {
  prepareRoutePreload,
  type PreparedRoutePreload,
} from '@microsoft/webui-router/preload.js';
import { routerConfig } from './router-config';

let activation: Promise<void> | undefined;
let prepared: PreparedRoutePreload | undefined;
const activate = () => {
  activation ??= Promise.all([
    import('./component-definitions'),
    import('@microsoft/webui-router'),
  ]).then(([, { Router }]) => {
    Router.start({ ...routerConfig, preload: prepared });
  });
  return activation;
};
const disposeHydration = installInteractionHydration({
  load: activate,
  onError: () => prepared?.destroy(),
});
try {
  prepared = prepareRoutePreload({
    onIntent: () => wakeInteractionHydration(),
  });
} catch (error) {
  disposeHydration();
  throw error;
}
(window as unknown as Record<string, unknown>).__testInteractionInstalled = true;
