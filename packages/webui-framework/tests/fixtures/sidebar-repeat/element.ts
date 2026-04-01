// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';
import {
  and,
  attrTarget,
  bindAttr,
  bindBoolAttr,
  bindTemplateAttr,
  bindText,
  dynamic,
  eq,
  nodePath,
  registerCompiledTemplate,
  repeat,
  slot,
  stringLiteral,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-sidebar-repeat', {
  h: `<link rel="stylesheet" href="/sidebar-repeat/sidebar-repeat.css" />
  <nav class="sidebar">
    <div class="nav-section">
      <a class="nav-item" data-nav="Dashboard" href="/">Dashboard</a>
      <a class="nav-item" data-nav="All Contacts" href="/contacts">All Contacts</a>
      <a class="nav-item" data-nav="Favorites" href="/favorites">Favorites</a>
    </div>
    <div class="nav-divider"></div>
    <div class="nav-section">
      <h3 class="nav-heading">Groups</h3>
      
    </div>
  </nav>`,
  attrs: [
    bindBoolAttr('data-active', eq('page', stringLiteral('dashboard'))),
    bindBoolAttr('data-active', eq('page', stringLiteral('contacts'))),
    bindBoolAttr('data-active', eq('page', stringLiteral('favorites'))),
  ],
  attrGroups: [
    attrTarget(nodePath(2, 1, 1), { startIndex: 0, bindingCount: 1 }),
    attrTarget(nodePath(2, 1, 3), { startIndex: 1, bindingCount: 1 }),
    attrTarget(nodePath(2, 1, 5), { startIndex: 2, bindingCount: 1 }),
  ],
  repeats: [repeat('groups', 'group', { blockIndex: 0 })],
  repeatSlots: [
    slot({ parent: nodePath(2, 5), before: 3 }),
  ],
  blocks: [{
    h: '<a class="nav-item nav-item-group"></a>',
    text: [
      bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('group')),
    ],
    attrs: [
      bindBoolAttr(
        'data-active',
        and(eq('page', stringLiteral('group')), eq('activeGroup', 'group')),
      ),
      bindAttr('data-nav', 'group'),
      bindTemplateAttr('href', '/groups/', dynamic('group')),
    ],
    attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 3 })],
  }],
});

export class TestSidebarRepeat extends WebUIElement {
  @observable page = 'dashboard';
  @observable activeGroup = '';
  @observable groups: string[] = ['work', 'family', 'friends', 'other'];

  syncGroups(): void {
    this.groups = ['work', 'family', 'friends', 'other'];
  }
}

TestSidebarRepeat.define('test-sidebar-repeat');

