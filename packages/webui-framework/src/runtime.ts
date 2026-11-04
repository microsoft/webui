// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { installTemplateElementRuntime } from './static-host.js';
import { activateTemplateDefinitions, installTemplateRegistry } from './template-registry.js';
import { installHydrationLifecycle } from './lifecycle.js';
import { runHydrationBatch } from './hydration-batch.js';
import { installStylesRuntime } from './style-installation.js';

/** Start rendering and component styling after importing component declarations. */
export function startRuntime(): void {
  installTemplateRegistry();
  installStylesRuntime();
  installHydrationLifecycle();
  if (runHydrationBatch(activateTemplateDefinitions)) {
    setTimeout(installTemplateElementRuntime, 0);
  }
}
