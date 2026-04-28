// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './demo-shell/demo-shell.js';

window.addEventListener('webui:hydration-complete', () => {
  // eslint-disable-next-line no-console
  console.log('[demo-shell] hydration complete');
});
