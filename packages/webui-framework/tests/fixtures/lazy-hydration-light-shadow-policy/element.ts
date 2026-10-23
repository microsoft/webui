// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import '../../../src/lazy-hydration-entry.js';
import { WebUIElement } from '../../../src/index.js';

startRuntime();

class LightOffscreenItem extends WebUIElement {
}
LightOffscreenItem.define('light-offscreen-item');
