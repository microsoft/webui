// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { installTemplateElementRuntime } from './static-host.js';
import { activateTemplateDefinitions, installTemplateRegistry } from './template-registry.js';
import { installHydrationLifecycle, runHydrationBatch } from './lifecycle.js';

/** Start selected rendering after module evaluation, including declarations in shared chunks. */
export function startRuntime(): void {
  installTemplateRegistry();
  installHydrationLifecycle();
  if (runHydrationBatch(activateTemplateDefinitions)) {
    setTimeout(installTemplateElementRuntime, 0);
  }
}
