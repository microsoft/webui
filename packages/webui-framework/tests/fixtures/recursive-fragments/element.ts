// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import '../../../src/lazy-hydration-entry.js';
import { observable, WebUIElement } from '../../../src/index.js';

export interface TreeItem {
  id: string;
  name: string;
  children: TreeItem[];
}

export class TestRecursiveTree extends WebUIElement {
  @observable title = '';
  @observable items: TreeItem[] = [];
  @observable selected = '';
  @observable focused = '';
  hydrations = 0;

  protected override hydratedCallback(): void {
    this.hydrations++;
  }

  select(id: string, name: string): void {
    this.selected = `${id}:${name}`;
  }

  recordFocus(id: string): void {
    this.focused = id;
  }
}
TestRecursiveTree.define('test-recursive-tree');

export class TestRecursiveLazy extends TestRecursiveTree {}
TestRecursiveLazy.define('test-recursive-lazy');

export class TestRecursiveLight extends WebUIElement {
  @observable title = '';
  @observable prefix = '';
  @observable suffix = '';
  @observable show = false;
  @observable tail: string[] = [];
  @observable html = '';
  @observable scalar: unknown = null;
  @observable lengthText = '';
  @observable invalidLength = false;
  @observable fallbackItems: Array<{ fallback?: string }> = [];
  @observable item = { fallback: '' };
}
TestRecursiveLight.define('test-recursive-light');

export class TestRecursiveTable extends WebUIElement {
  @observable items: TreeItem[] = [];
  @observable title = '';
  @observable show = true;
  @observable columns = 1;
}
TestRecursiveTable.define('test-recursive-table');

export class TestRecursiveUnknown extends WebUIElement {
  @observable pulse = 0;

  increment(): void {
    this.pulse++;
  }
}
TestRecursiveUnknown.define('test-recursive-unknown');
