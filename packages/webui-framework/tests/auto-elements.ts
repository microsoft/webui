// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Shared fixture bootstrap for HTML-only auto-element tests.
 *
 * These fixtures intentionally have no `element.ts`; they opt into the
 * HTML-only runtime explicitly so authored components do not pay for it.
 */

import { installAutoElementRuntime } from '../src/auto-element.js';

installAutoElementRuntime();
