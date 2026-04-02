// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '../../../src/index.js';
import {
  attrTarget,
  bindAttr,
  bindBoolAttr,
  bindEvent,
  bindText,
  dynamic,
  identifier,
  nodePath,
  registerCompiledTemplate,
  slot,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-attr', {
  h: '<button class="noop">Toggle</button><a class="logo" href="/">Home</a><span class="label"></span><span class="display"></span><a class="cta">Open</a><div class="bool-target"></div><input class="bool-check" type="checkbox" />',
  text: [
    bindText(slot({ parent: nodePath(2), before: 0 }), dynamic('label')),
    bindText(slot({ parent: nodePath(3), before: 0 }), dynamic('displayValue')),
  ],
  attrs: [
    bindAttr('href', 'ctaHref'),
    bindBoolAttr('data-active', identifier('isActive')),
    bindBoolAttr('checked', identifier('isActive')),
  ],
  attrGroups: [
    attrTarget(nodePath(4), { startIndex: 0, bindingCount: 1 }),
    attrTarget(nodePath(5), { startIndex: 1, bindingCount: 1 }),
    attrTarget(nodePath(6), { startIndex: 2, bindingCount: 1 }),
  ],
  events: [
    bindEvent('click', 'noop', false, nodePath(0)),
    bindEvent('click', 'noop', false, nodePath(1)),
  ],
});

export class TestAttr extends WebUIElement {
  @attr label = 'Status';
  @attr({ attribute: 'display-value' }) displayValue = 'Ready';
  @attr({ attribute: 'cta-href' }) ctaHref = '/checkout';
  @attr({ mode: 'boolean', attribute: 'is-active' }) isActive = false;

  noop(): void {}
}

TestAttr.define('test-attr');

