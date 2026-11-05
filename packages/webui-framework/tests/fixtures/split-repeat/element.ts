// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { WebUIElement, observable } from '../../../src/index.js';

startRuntime();

interface SplitRepeatItem {
  label: string;
}

export class TestSplitRepeat extends WebUIElement {
  @observable primaryItems: SplitRepeatItem[] = [{ label: 'Seed Alpha' }, { label: 'Seed Beta' }];
  @observable secondaryItems: SplitRepeatItem[] = [{ label: 'Seed One' }, { label: 'Seed Two' }];

  loadItems(): void {
    this.primaryItems = [{ label: 'Alpha' }, { label: 'Beta' }];
    this.secondaryItems = [{ label: 'One' }, { label: 'Two' }, { label: 'Three' }];
  }
}

TestSplitRepeat.define('test-split-repeat');

