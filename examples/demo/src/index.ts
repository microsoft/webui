// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
import './demo-shell/demo-shell.js';

startRuntime();

window.addEventListener('webui:hydration-complete', () => {
  // eslint-disable-next-line no-console
  console.log('[demo-shell] hydration complete');
});
