// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

interface MultiRepeatItem {
  title: string;
  href: string;
  active: string;
}

export class TestMultiRepeat extends WebUIElement {
  @observable items: MultiRepeatItem[] = [
    { title: 'Alpha', href: '/alpha', active: 'true' },
    { title: 'Beta', href: '/beta', active: 'false' },
  ];
}

TestMultiRepeat.define('test-multi-repeat');
