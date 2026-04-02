// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';
import {
  attrTarget,
  bindAttr,
  bindEvent,
  bindProp,
  bindTemplateAttr,
  bindText,
  dynamic,
  nodePath,
  registerCompiledTemplate,
  repeat,
  slot,
} from '@microsoft/webui-test-support';

interface NavCategory {
  handle: string;
  title: string;
  activeClass: string;
}

registerCompiledTemplate('test-state-seed-shell', {
  h: '<div class="page"></div><nav class="groups"></nav><nav class="categories"></nav>',
  text: [
    bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('page')),
  ],
  repeats: [
    repeat('groups', 'group', { blockIndex: 0, slot: { parent: nodePath(1), before: 0 } }),
    repeat('navCategories', 'cat', { blockIndex: 1, slot: { parent: nodePath(2), before: 0 } }),
  ],
  blocks: [
    {
      h: '<a class="group-link"></a>',
      text: [
        bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('group')),
      ],
      attrs: [
        bindAttr('data-nav', 'group'),
        bindTemplateAttr('href', '/groups/', dynamic('group')),
      ],
      attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 2 })],
    },
    {
      h: '<a class="category-link"></a>',
      text: [
        bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('cat.title')),
      ],
      attrs: [
        bindTemplateAttr('class', 'category-link ', dynamic('cat.activeClass')),
        bindTemplateAttr('href', '/search/', dynamic('cat.handle')),
      ],
      attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 2 })],
    },
  ],
});

registerCompiledTemplate('test-state-seed', {
  h: '<h1 class="title"></h1><test-state-seed-shell></test-state-seed-shell><button class="add-group">Add Group</button><button class="add-category">Add Category</button>',
  text: [
    bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('title')),
  ],
  attrs: [
    bindAttr('page', 'page'),
    bindProp('groups', 'groups'),
    bindProp('navCategories', 'navCategories'),
  ],
  attrGroups: [attrTarget(nodePath(1), { startIndex: 0, bindingCount: 3 })],
  events: [
    bindEvent('click', 'addGroup', false, nodePath(2)),
    bindEvent('click', 'addCategory', false, nodePath(3)),
  ],
});

export class TestStateSeedShell extends WebUIElement {
  @attr page = '';
  @observable groups: string[] = ['work', 'family'];
  @observable navCategories: NavCategory[] = [
    { handle: 'featured', title: 'Featured', activeClass: 'active' },
    { handle: 'sale', title: 'Sale', activeClass: '' },
  ];
}

TestStateSeedShell.define('test-state-seed-shell');

export class TestStateSeed extends WebUIElement {
  @observable title = 'SSR Title';
  @observable page = 'dashboard';
  @observable groups: string[] = ['work', 'family'];
  @observable navCategories: NavCategory[] = [
    { handle: 'featured', title: 'Featured', activeClass: 'active' },
    { handle: 'sale', title: 'Sale', activeClass: '' },
  ];

  addGroup(): void {
    this.groups = [...this.groups, 'travel'];
  }

  addCategory(): void {
    this.navCategories = [...this.navCategories, {
      handle: 'travel',
      title: 'Travel',
      activeClass: '',
    }];
  }
}

TestStateSeed.define('test-state-seed');

