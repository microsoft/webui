// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';

interface SidebarGroup {
  className: string;
  label: string;
  slug: string;
}

export class TestSidebarRepeat extends WebUIElement {
  @attr page = 'dashboard';
  @attr activeGroup = '';
  @observable groups: SidebarGroup[] = [
    { className: 'nav-item nav-item-group', label: 'work', slug: 'work' },
    { className: 'nav-item nav-item-group', label: 'family', slug: 'family' },
    { className: 'nav-item nav-item-group', label: 'friends', slug: 'friends' },
    { className: 'nav-item nav-item-group', label: 'other', slug: 'other' },
  ];

  syncGroups(): void {
    this.groups = [
      { className: 'nav-item nav-item-group', label: 'work', slug: 'work' },
      { className: 'nav-item nav-item-group', label: 'family', slug: 'family' },
      {
        className: 'nav-item nav-item-group',
        label: 'friends',
        slug: 'friends',
      },
      { className: 'nav-item nav-item-group', label: 'other', slug: 'other' },
    ];
  }
}

TestSidebarRepeat.define('test-sidebar-repeat');
