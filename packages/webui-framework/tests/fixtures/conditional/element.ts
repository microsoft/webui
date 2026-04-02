// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';
import {
  attrTarget,
  bindBoolAttr,
  bindEvent,
  bindText,
  dynamic,
  identifier,
  nodePath,
  registerCompiledTemplate,
  slot,
  when,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-conditional', {
  h: '<button class="toggle">Toggle</button>',
  attrs: [bindBoolAttr('disabled', identifier('busy'))],
  attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 1 })],
  conditionals: [when(identifier('open'), { blockIndex: 0, slot: { before: 1 } })],
  blocks: [{
    h: '<span class="details"></span>',
    text: [
      bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('details')),
    ],
  }],
  events: [bindEvent('click', 'toggleOpen', false, nodePath(0))],
});

registerCompiledTemplate('test-conditional-client', {
  h: '<button class="toggle">Toggle</button>',
  attrs: [bindBoolAttr('disabled', identifier('busy'))],
  attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 1 })],
  conditionals: [when(identifier('open'), { blockIndex: 0, slot: { before: 1 } })],
  blocks: [{
    h: '<span class="details"></span>',
    text: [
      bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('details')),
    ],
  }],
  events: [bindEvent('click', 'toggleOpen', false, nodePath(0))],
});

registerCompiledTemplate('test-conditional-detached', {
  h: '<button class="toggle">Toggle</button>',
  attrs: [bindBoolAttr('disabled', identifier('busy'))],
  attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 1 })],
  conditionals: [when(identifier('open'), { blockIndex: 0, slot: { before: 1 } })],
  blocks: [{
    h: '<span class="details"></span>',
    text: [
      bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('details')),
    ],
  }],
  events: [bindEvent('click', 'toggleOpen', false, nodePath(0))],
});

export class TestConditional extends WebUIElement {
  @observable open = true;
  @observable busy = false;
  @observable details = 'Details';

  toggleOpen(): void {
    this.open = !this.open;
  }
}

TestConditional.define('test-conditional');

export class TestConditionalClient extends WebUIElement {
  @observable open = true;
  @observable busy = false;
  @observable details = 'Details';

  toggleOpen(): void {
    this.open = !this.open;
  }
}

TestConditionalClient.define('test-conditional-client');

