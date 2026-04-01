// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';
import {
  attrTarget,
  bindAttr,
  bindBoolAttr,
  bindEvent,
  bindText,
  dynamic,
  eq,
  identifier,
  nodePath,
  neq,
  registerCompiledTemplate,
  repeat,
  slot,
  when,
  stringLiteral,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-repeat-conditional', {
  h: '<div class="controls"><button class="load">Load</button><button class="switch">Switch</button></div><ul class="items"></ul>',
  repeats: [repeat('items', 'item', { blockIndex: 0 })],
  repeatSlots: [slot({ parent: nodePath(1), before: 0 })],
  blocks: [
    {
      h: '<li></li>',
      conditionals: [
        when(eq('item.activeClass', stringLiteral('active')), { blockIndex: 1 }),
        when(neq('item.activeClass', stringLiteral('active')), { blockIndex: 2 }),
      ],
      conditionSlots: [
        slot({ parent: nodePath(0), before: 0, order: 0 }),
        slot({ parent: nodePath(0), before: 0, order: 1 }),
      ],
    },
    {
      h: '<p class="current"></p>',
      text: [
        bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('item.title')),
      ],
      attrs: [bindAttr('data-href', 'item.href')],
      attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 1 })],
    },
    {
      h: '<button class="link"></button>',
      text: [
        bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('item.title')),
      ],
      attrs: [
        bindAttr('data-href', 'item.href'),
        bindBoolAttr('disabled', identifier('item.disabled')),
      ],
      attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 2 })],
    },
  ],
  events: [
    bindEvent('click', 'loadItems'),
    bindEvent('click', 'switchItems'),
  ],
  eventTargets: [nodePath(0, 0), nodePath(0, 1)],
});

interface RepeatConditionalItem {
  title: string;
  href: string;
  activeClass: string;
  disabled: boolean;
}

export class TestRepeatConditional extends WebUIElement {
  @observable items: RepeatConditionalItem[] = [
    {
      title: 'Shirts',
      href: '/search/shirts',
      activeClass: 'active',
      disabled: false,
    },
    {
      title: 'Headwear',
      href: '/search/headwear',
      activeClass: '',
      disabled: false,
    },
    {
      title: 'Archived',
      href: '/search/archived',
      activeClass: '',
      disabled: true,
    },
  ];

  loadItems(): void {
    this.items = [
      {
        title: 'Shirts',
        href: '/search/shirts',
        activeClass: 'active',
        disabled: false,
      },
      {
        title: 'Headwear',
        href: '/search/headwear',
        activeClass: '',
        disabled: false,
      },
      {
        title: 'Archived',
        href: '/search/archived',
        activeClass: '',
        disabled: true,
      },
    ];
  }

  switchItems(): void {
    this.items = [
      {
        title: 'Shirts',
        href: '/search/shirts',
        activeClass: '',
        disabled: false,
      },
      {
        title: 'Headwear',
        href: '/search/headwear',
        activeClass: 'active',
        disabled: false,
      },
      {
        title: 'Archived',
        href: '/search/archived',
        activeClass: '',
        disabled: false,
      },
    ];
  }
}

TestRepeatConditional.define('test-repeat-conditional');

