// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { WebUIElement, attr, observable } from '../../../src/index.js';

startRuntime();

export class TestForeignContent extends WebUIElement {
  @attr encoding = 'text/html';
  @observable ready = false;
  @observable show = false;
  @observable label = 'Native content';
  @observable points: { id: number; x: number }[] = [];
  @observable links: { id: string; href: string; label: string }[] = [];

  protected hydratedCallback(): void {
    this.ready = true;
  }

  toggle(): void {
    this.show = !this.show;
    this.points = this.show ? [{ id: 1, x: 20 }, { id: 2, x: 50 }] : [];
    this.links = this.show ? [{ id: 'shared', href: '#context', label: 'Shared link' }] : [];
  }

  reverseItems(): void {
    this.points = [...this.points].reverse();
  }
}

TestForeignContent.define('test-foreign-content');

export class TestForeignChild extends WebUIElement {
  static constructions = 0;
  @attr label = '';

  constructor() {
    super();
    TestForeignChild.constructions++;
  }
}

TestForeignChild.define('test-foreign-child');

declare global {
  interface HTMLElementTagNameMap {
    'test-foreign-content': TestForeignContent;
    'test-foreign-child': TestForeignChild;
  }
}
