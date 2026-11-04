// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  installInteractionHydration,
  wakeInteractionHydration,
} from '@microsoft/webui-framework/interaction-hydration.js';
import {
  routePreload,
} from '@microsoft/webui-delivery/router.js';
import { routerConfig } from './router-config';

let activation: Promise<void> | undefined;
const preload = routePreload({ onIntent: () => wakeInteractionHydration() });
const activate = () => {
  activation ??= Promise.all([
    import('./component-definitions'),
    import('@microsoft/webui-delivery/router.js'),
    import('@microsoft/webui-delivery/resources.js'),
  ]).then(([, { WebUIRouter, navigationCache, routeBoundaries, viewTransitions }, { ndjson }]) => {
    new WebUIRouter().start({
      ...routerConfig, preload, cache: navigationCache(), transport: ndjson(),
      boundaries: routeBoundaries, transitions: viewTransitions,
    });
  });
  return activation;
};
const disposeHydration = installInteractionHydration({
  load: activate,
  onError: () => preload.destroy(),
});
try {
  preload.prepare();
} catch (error) {
  disposeHydration();
  throw error;
}
(window as unknown as Record<string, unknown>).__testInteractionInstalled = true;
