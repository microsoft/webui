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
  conditionals: [when(identifier('open'), { blockIndex: 0 })],
  conditionSlots: [slot({ before: 1 })],
  blocks: [{
    h: '<span class="details"></span>',
    text: [
      bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('details')),
    ],
  }],
  events: [bindEvent('click', 'toggleOpen')],
  eventTargets: [nodePath(0)],
});

registerCompiledTemplate('test-conditional-client', {
  h: '<button class="toggle">Toggle</button>',
  attrs: [bindBoolAttr('disabled', identifier('busy'))],
  attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 1 })],
  conditionals: [when(identifier('open'), { blockIndex: 0 })],
  conditionSlots: [slot({ before: 1 })],
  blocks: [{
    h: '<span class="details"></span>',
    text: [
      bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('details')),
    ],
  }],
  events: [bindEvent('click', 'toggleOpen')],
  eventTargets: [nodePath(0)],
});

registerCompiledTemplate('test-conditional-detached', {
  h: '<button class="toggle">Toggle</button>',
  attrs: [bindBoolAttr('disabled', identifier('busy'))],
  attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 1 })],
  conditionals: [when(identifier('open'), { blockIndex: 0 })],
  conditionSlots: [slot({ before: 1 })],
  blocks: [{
    h: '<span class="details"></span>',
    text: [
      bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('details')),
    ],
  }],
  events: [bindEvent('click', 'toggleOpen')],
  eventTargets: [nodePath(0)],
});

interface InternalTemplateInstance {
  nodes: Node[];
}

interface InternalWebUIElement {
  $createTemplateInstance(
    nodes: Node[],
    meta?: unknown,
    scope?: unknown,
  ): InternalTemplateInstance;
  $fragmentFromNodes(nodes: Node[]): DocumentFragment;
  $bindRefs(nodes: Node[]): void;
  $clean(sr: ShadowRoot): void;
  $root: InternalTemplateInstance | null;
  $hydrated: boolean;
  $ready: boolean;
}

export class TestConditional extends WebUIElement {
  @observable open = true;
  @observable busy = false;
  @observable details = '';

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

export class TestConditionalDetached extends WebUIElement {
  @observable open = true;
  @observable busy = false;
  @observable details = 'Details';

  connectedCallback(): void {
    if (this.shadowRoot) {
      return;
    }

    const internal = this as unknown as InternalWebUIElement;
    const templates = window as Window & {
      __webui_templates?: Record<string, unknown>;
    };
    const meta = templates.__webui_templates?.['test-conditional-detached'];
    if (!meta) {
      throw new Error('missing test-conditional-detached template metadata');
    }

    const template = document.createElement('template');
    template.innerHTML = '<button class="toggle" data-w-b-0 data-ev="0">Toggle</button><!--w-b:start:0:if-1--><span class="details"><!--w-b:start:0:details-->Details<!--w-b:end:0:details--></span><!--w-b:end:0:if-1-->';

    const shadowRoot = this.attachShadow({ mode: 'open' });
    const nodes = Array.from(template.content.childNodes);
    const root = internal.$createTemplateInstance(nodes, meta);
    internal.$root = root;
    internal.$bindRefs(root.nodes);
    shadowRoot.appendChild(internal.$fragmentFromNodes(root.nodes));
    internal.$hydrated = true;
    internal.$clean(shadowRoot);
    internal.$ready = true;
    this.$update();
  }

  toggleOpen(): void {
    this.open = !this.open;
  }
}

TestConditionalDetached.define('test-conditional-detached');

