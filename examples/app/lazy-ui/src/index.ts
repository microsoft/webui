// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
import '@microsoft/webui-framework/template-cache.js';
import '@microsoft/webui-framework/lazy.js';
import './lazy-popover/lazy-popover.js';
import './lazy-dialog/lazy-dialog.js';
import './lazy-workspace/lazy-workspace.js';

startRuntime();
