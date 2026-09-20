// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Todo-fast entry point — bootstraps FAST hydration.
 *
 * The server pre-renders HTML with hydration markers via `webui build --plugin=fast-v3`.
 * This script:
 *   1. Enables FAST hydration for pre-rendered shadow DOM
 *   2. Registers custom elements (todo-app, todo-item) with declarative templates
 */

performance.mark('todo-hydration-started');

import { enableHydration } from '@microsoft/fast-element/hydration.js';

const hydration = enableHydration();

// fast-element 3.0 replaced the `hydrationComplete` callback with a
// `whenHydrated()` promise that resolves once the active hydration batch
// finishes.
void hydration.whenHydrated().then(() => {
  performance.measure('todo-hydration-completed', 'todo-hydration-started');
  console.log('Hydration complete!');
});

// Register custom elements after hydration is enabled.
void Promise.all([
  import('./todo-app/todo-app.js'),
  import('./todo-item/todo-item.js'),
]);
