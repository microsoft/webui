// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

interface RepeatConditionalItem {
  title: string;
  href: string;
  activeClass: string;
  disabled: boolean;
}

export class TestRepeatConditional extends WebUIElement {
  @observable selectedTitle = '';

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

  selectItem(title: string): void {
    this.selectedTitle = title;
  }
}

TestRepeatConditional.define('test-repeat-conditional');
