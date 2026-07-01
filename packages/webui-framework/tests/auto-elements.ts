// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Shared fixture bootstrap for HTML-only auto-element tests.
 *
 * These fixtures intentionally have no `element.ts`; importing the framework
 * root is enough to install the auto-element runtime and prove scriptless
 * templates hydrate without authored component stubs.
 */

import '../src/index.js';
