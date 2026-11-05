// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { WebUIElement, observable } from '../../../src/index.js';

startRuntime();

interface Option {
  title: string;
  active: boolean;
}

export class TestTextOnlyRepeat extends WebUIElement {
  @observable options: Option[] = [];

  onUpdate(): void {
    // Shift active from first to second option
    this.options = this.options.map((opt, i) => ({
      ...opt,
      active: i === 1,
    }));
  }

  clearOptions(): void {
    this.options = [];
  }
}

TestTextOnlyRepeat.define('test-text-only-repeat');
