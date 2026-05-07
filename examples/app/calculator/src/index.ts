// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Calculator hydration entry point.
 *
 * The server pre-renders HTML with hydration markers via `webui build --plugin=fast-v3`.
 * This script enables FAST hydration and registers custom elements with
 * declarative templates.
 */

performance.mark('calc-hydration-started');

import { enableHydration } from '@microsoft/fast-element/hydration.js';

enableHydration({
  hydrationComplete() {
    performance.measure('calc-hydration-completed', 'calc-hydration-started');
    console.log('Calculator hydration complete!');
  },
});

// Register custom elements after hydration is enabled.
void Promise.all([
  import('./calc-app/calc-app.js'),
  import('./calc-display/calc-display.js'),
  import('./calc-button/calc-button.js'),
]);
