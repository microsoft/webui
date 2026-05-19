// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';

export class TestSidebarRepeat extends WebUIElement {
  @attr page = 'dashboard';
  @attr activeGroup = '';
  @observable groups: string[] = ['work', 'family', 'friends', 'other'];

  syncGroups(): void {
    this.groups = ['work', 'family', 'friends', 'other'];
  }
}

TestSidebarRepeat.define('test-sidebar-repeat');
