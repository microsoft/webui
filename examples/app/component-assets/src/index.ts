// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Static component asset example.
 *
 * The initial bundle registers <app-shell> and opts into the HTML-only runtime
 * for shared static component assets.
 */

import { installAutoElementRuntime } from '@microsoft/webui-framework/auto-element.js';
import './app-shell/app-shell.js';

installAutoElementRuntime();
