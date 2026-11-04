// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { beginHydrationBatch, hydrationEnd } from './lifecycle.js';

/** Keep synchronous definition work in one cohort without starting a timing mark. */
export function runHydrationBatch<T>(work: () => T): T {
  beginHydrationBatch();
  try {
    return work();
  } finally {
    hydrationEnd();
  }
}
