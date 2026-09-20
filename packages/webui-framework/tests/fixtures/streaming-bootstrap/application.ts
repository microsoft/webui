// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './src/test-stream-counter/test-stream-counter.js';
import './src/test-stream-parent/test-stream-parent.js';

window.__streamingApplicationStarted = true;

declare global {
  interface Window {
    __streamingApplicationStarted?: boolean;
    __streamingActivationOrder?: string[];
    __streamingCompletions?: number;
  }
}
