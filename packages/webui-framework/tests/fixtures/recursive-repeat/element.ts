// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

interface TreeNode {
  id: string;
  name: string;
  children?: TreeNode[];
}

export class TestRecursiveTree extends WebUIElement {
  @observable items: TreeNode[] = [];
  @observable prefix = 'Node: ';
  @observable child = { name: 'Component scope' };
  @observable selected = 'none';
  @observable clicks = 0;

  select(id: string): void {
    this.selected = id;
    this.clicks += 1;
  }
}

export class TestRecursiveForward extends WebUIElement {
  @observable forwardItems: TreeNode[] = [];
  @observable none: TreeNode[] = [];
  @observable forwardSelected = 'none';

  selectForward(id: string): void {
    this.forwardSelected = id;
  }
}

export class TestRecursiveIndependent extends WebUIElement {
  @observable independentItems: TreeNode[] = [];
}

export class TestRecursiveMutual extends WebUIElement {
  @observable mutualItems: TreeNode[] = [];
  @observable none: TreeNode[] = [];
}

TestRecursiveTree.define('test-recursive-tree');
TestRecursiveForward.define('test-recursive-forward');
TestRecursiveIndependent.define('test-recursive-independent');
TestRecursiveMutual.define('test-recursive-mutual');
