// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Calculator hydration entry point.
 *
 * The server pre-renders HTML with hydration markers via `webui build --plugin=fast`.
 * This script registers custom elements, configures FAST-HTML observation maps,
 * and defines <f-template> to trigger hydration.
 */

performance.mark('calc-hydration-started');

import { TemplateElement } from '@microsoft/fast-html';

// Side-effect imports register custom elements
import './calc-app/calc-app.js';
import './calc-display/calc-display.js';
import './calc-button/calc-button.js';

// Configure hydration
TemplateElement.options({
  'calc-app': { observerMap: 'all' },
  'calc-display': { observerMap: 'all' },
  'calc-button': { observerMap: 'all' },
}).config({
  hydrationComplete() {
    performance.measure('calc-hydration-completed', 'calc-hydration-started');
    console.log('Calculator hydration complete!');
  },
}).define({
  name: 'f-template',
});
