// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';

interface NavCategory {
  handle: string;
  title: string;
  activeClass: string;
}

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

