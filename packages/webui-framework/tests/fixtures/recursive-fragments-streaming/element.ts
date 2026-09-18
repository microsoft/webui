// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import '../../../src/streaming-entry.js';
import { observable, WebUIElement } from '../../../src/index.js';

interface TreeItem {
  id: string;
  name: string;
  children?: TreeItem[];
}

export class TestStreamedRecursiveTree extends WebUIElement {
  @observable title = '';
  @observable items: TreeItem[] = [];
  @observable selected = '';
  hydrations = 0;
  tailPresentAtHydration = false;

  protected override hydratedCallback(): void {
    this.hydrations++;
    this.tailPresentAtHydration = document.querySelector('footer') !== null;
  }

  select(name: string): void {
    this.selected = name;
  }
}
TestStreamedRecursiveTree.define('test-streamed-recursive-tree');

export class TestStreamedFragmentCapture extends WebUIElement {
  @observable source: { name: string; branches?: TreeItem[] } = { name: '' };
  @observable title = '';
  @observable selected = '';
  @observable showCapturedTree = true;

  select(name: string, title: string): void {
    this.selected = `${name}/${title}`;
  }
}
TestStreamedFragmentCapture.define('test-streamed-fragment-capture');

export class TestStreamedFragmentProps extends WebUIElement {
  @observable model = { name: '' };
  @observable title = '';
  @observable selected = '';
  hydrations = 0;

  protected override hydratedCallback(): void {
    this.hydrations++;
  }

  select(name: string, title: string): void {
    this.selected = `${name}/${title}`;
  }
}
TestStreamedFragmentProps.define('test-streamed-fragment-props');
