// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { WebUIElement, observable } from '../../../src/index.js';

startRuntime();

export class TestNullishAttribute extends WebUIElement {
  @observable cases: { name: string; value?: string | number | boolean | null }[] = [];
  @observable constraint: string | number | boolean | null | undefined = null;
  @observable inputValue: string | null = 'abc';
  @observable label = 'Before';

  rename(): void {
    this.label = 'After';
  }
}

TestNullishAttribute.define('test-nullish-attribute');
