// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { WebUIElement, observable } from '../../../src/index.js';

startRuntime();

export class TestConditional extends WebUIElement {
  @observable open = true;
  @observable busy = false;
  @observable details = 'Details';
  @observable count = 1;
  @observable empty = false;

  toggleOpen(): void {
    this.open = !this.open;
  }
}

TestConditional.define('test-conditional');

export class TestConditionalClient extends WebUIElement {
  @observable open = true;
  @observable busy = false;
  @observable details = 'Details';

  toggleOpen(): void {
    this.open = !this.open;
  }
}

TestConditionalClient.define('test-conditional-client');

export class TestConditionalHydrationRanges extends WebUIElement {
  @observable clientOnlyOpen = true;
  @observable outerOpen = true;
  @observable innerOpen = false;
  @observable snapshotOpen = false;
  @observable snapshotItems: string[] = [];

  protected $shouldApplySSRBootstrapState(): boolean {
    return false;
  }

  toggleClientOnlyOpen(): void {
    this.clientOnlyOpen = !this.clientOnlyOpen;
  }

  toggleOuterOpen(): void {
    this.outerOpen = !this.outerOpen;
  }
}

TestConditionalHydrationRanges.define('test-conditional-hydration-ranges');

export class TestConditionalInterleaved extends WebUIElement {
  @observable showStats = true;
  @observable compact = false;
  @observable value = '';
}

TestConditionalInterleaved.define('test-conditional-interleaved');

export class TestConditionalSiblingAfterNested extends WebUIElement {
  @observable outer = true;
  @observable inner = false;
  @observable tail = true;
}

TestConditionalSiblingAfterNested.define('test-conditional-sibling-after-nested');

export class TestConditionalBlockEscape extends WebUIElement {
  @observable escOuter = true;
  @observable escInner = true;
  @observable escAfter = true;
}

TestConditionalBlockEscape.define('test-conditional-block-escape');

export class TestTruthinessHost extends WebUIElement {
  @observable emptyItems: string[] = [];
  @observable emptyActions: { trailing: string[] } = { trailing: [] };
  @observable emptyRecord: { label?: string } = {};
  @observable populatedItems = ['Item'];
  @observable populatedActions = { trailing: ['Action'] };
  @observable populatedRecord = { label: 'Record' };
}

TestTruthinessHost.define('test-truthiness-host');

export class TestConditionTruthiness extends WebUIElement {
  @observable items: string[] = [];
  @observable actions: { trailing: string[] } = { trailing: [] };
  @observable record: { label?: string } = {};
  @observable phase = 'initial';
  @observable falseValue = false;
  @observable zero = 0;
  @observable emptyText = '';
  @observable nullValue: null = null;
  @observable missing: boolean | undefined = undefined;
  @observable trueValue = true;

  populate(): void {
    this.items = ['Item'];
    this.actions = { trailing: ['Action'] };
    this.record = { label: 'Record' };
    this.phase = 'populated';
  }

  clear(): void {
    this.items = [];
    this.actions = { trailing: [] };
    this.record = {};
    this.phase = 'empty';
  }
}

TestConditionTruthiness.define('test-condition-truthiness');
