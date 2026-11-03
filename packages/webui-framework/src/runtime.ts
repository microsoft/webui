// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { installTemplateElementRuntime } from './static-host.js';
import { activateTemplateDefinitions, installTemplateRegistry } from './template-registry.js';
import { installHydrationLifecycle } from './lifecycle.js';
import { runHydrationBatch } from './hydration-batch.js';

/** Start selected rendering after module evaluation, including declarations in shared chunks. */
export function startRuntime(): void {
  installTemplateRegistry();
  installHydrationLifecycle();
  if (runHydrationBatch(activateTemplateDefinitions)) {
    setTimeout(installTemplateElementRuntime, 0);
  }
}
