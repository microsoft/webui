// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
/**
 * Static component asset example.
 *
 * The initial bundle registers <app-shell>. The lazy component module is loaded
 * only when its compiled asset is requested.
 */

import './app-shell/app-shell.js';

startRuntime();
