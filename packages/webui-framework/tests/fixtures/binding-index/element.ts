// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';
import type { ScopeFrame } from '../../../src/element/types.js';

export class TestBindingIndex extends WebUIElement {
  @observable record = { first: 'Ada', last: 'Lovelace', active: true, allowed: true };
  @observable suffix = 'initial';
  @observable marker = 'Visible';
  @observable items = [{ label: 'one' }, { label: 'two' }];
  @observable locals = [{ first: 'Local', last: 'record' }];
  external = { first: 'External', last: 'initial' };
  extra = 'outside';
  reads: Record<string, number> = {};
  callbacks: string[] = [];
  reenter = false;

  override $resolveValue(path: string, scope?: ScopeFrame): unknown {
    this.reads[path] = (this.reads[path] ?? 0) + 1;
    return super.$resolveValue(path, scope);
  }

  recordChanged(): void {
    this.callbacks.push('record');
    if (this.reenter) this.suffix = 'reviewed';
  }

  suffixChanged(): void {
    this.callbacks.push('suffix');
  }

  rename(): void {
    this.reads = {};
    this.callbacks = [];
    this.external.last = 'updated';
    this.record = { ...this.record, first: 'Grace', last: 'Hopper' };
  }

  changeSuffix(): void {
    this.reads = {};
    this.callbacks = [];
    this.suffix = 'updated';
  }

  renameReentrant(): void {
    this.reenter = true;
    this.rename();
  }

  changeLocals(): void {
    this.reads = {};
    this.callbacks = [];
    this.locals = [{ first: 'Changed', last: 'local' }];
  }
}

TestBindingIndex.define('test-binding-index');
