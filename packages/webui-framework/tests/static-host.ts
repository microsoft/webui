// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Shared fixture bootstrap for scriptless dormant-host tests.
 *
 * These fixtures intentionally have no `element.ts`; importing the framework
 * runtime entry proves they do not need empty component stubs.
 */

import { startRuntime } from '../src/runtime.js';
import '../src/styles-entry.js';

startRuntime();
