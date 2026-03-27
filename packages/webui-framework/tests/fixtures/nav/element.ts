// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';
import {
  attrTarget,
  bindAttr,
  bindEvent,
  bindTemplateAttr,
  bindText,
  dynamic,
  nodePath,
  registerCompiledTemplate,
  repeat,
  slot,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-nav', {
  h: '<div class="controls"><button class="sync">Sync groups</button></div><nav class="sidebar"><div class="nav-section primary"><a class="nav-link" data-nav="Dashboard" href="/">Dashboard</a><a class="nav-link" data-nav="All Contacts" href="/contacts">All Contacts</a><a class="nav-link" data-nav="Favorites" href="/favorites">Favorites</a></div><div class="nav-section groups"><h3>Groups</h3></div></nav>',
  repeats: [repeat('groups', 'group', { blockIndex: 0 })],
  repeatSlots: [
    slot({ parent: nodePath(1, 1), before: 1 }),
  ],
  blocks: [{
    h: '<a class="nav-link nav-link-group"></a>',
    text: [
      bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('group')),
    ],
    attrs: [
      bindAttr('data-nav', 'group'),
      bindTemplateAttr('href', '/groups/', dynamic('group')),
    ],
    attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 2 })],
  }],
  events: [bindEvent('click', 'syncGroups')],
  eventTargets: [nodePath(0, 0)],
});

export class TestNav extends WebUIElement {
  @observable groups: string[] = [];

  syncGroups(): void {
    this.groups = ['work', 'family', 'friends', 'other'];
  }
}

TestNav.define('test-nav');

