// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '../../../src/index.js';
import {
  attrTarget,
  bindAttr,
  bindEvent,
  bindText,
  dynamic,
  nodePath,
  registerCompiledTemplate,
  slot,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-attr', {
  h: '<button class="noop">Toggle</button><a class="logo" href="/">Home</a><span class="label"></span><span class="display"></span><a class="cta">Open</a>',
  text: [
    bindText(slot({ parent: nodePath(2), before: 0 }), dynamic('label')),
    bindText(slot({ parent: nodePath(3), before: 0 }), dynamic('displayValue')),
  ],
  attrs: [bindAttr('href', 'ctaHref')],
  attrGroups: [attrTarget(nodePath(4), { startIndex: 0, bindingCount: 1 })],
  events: [
    bindEvent('click', 'noop'),
    bindEvent('click', 'noop'),
  ],
  eventTargets: [nodePath(0), nodePath(1)],
});

export class TestAttr extends WebUIElement {
  @attr label = 'Status';
  @attr({ attribute: 'display-value' }) displayValue = 'Ready';
  @attr({ attribute: 'cta-href' }) ctaHref = '/checkout';

  noop(): void {}
}

TestAttr.define('test-attr');

