// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Shared fixture bootstrap for HTML-only static-host tests.
 *
 * These fixtures intentionally have no `element.ts`; they import the framework
 * root to prove HTML-only components do not need a custom element stub.
 */

import '../src/index.js';
