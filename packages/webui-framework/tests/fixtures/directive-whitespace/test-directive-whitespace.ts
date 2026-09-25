// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '../../../src/index.js';

export interface WhitespaceItem {
  id: string;
  label: string;
  visible: boolean;
  detail: boolean;
  children: { id: string; label: string }[];
}

export class TestDirectiveWhitespace extends WebUIElement {
  @observable enabled = true;
  @observable ready = true;
  @observable label = 'Action';
  @observable selected = '';
  @observable items: WhitespaceItem[] = [];
  @observable message = { enabled: true, ready: true, label: 'Action' };

  select(id: string): void {
    this.selected = id;
  }

  replaceUnchanged(): void {
    this.message = { ...this.message };
    this.items = this.items.map(item => ({
      ...item,
      children: item.children.map(child => ({ ...child })),
    }));
    this.$flushUpdates();
  }
}

TestDirectiveWhitespace.define('test-directive-whitespace');

declare global {
  interface HTMLElementTagNameMap {
    'test-directive-whitespace': TestDirectiveWhitespace;
  }

  interface Window {
    __directiveWhitespaceSsr: Element[];
    __directiveWhitespaceSsrParents: (Node | null)[];
    __directiveWhitespaceKeyedSsr: Map<string, Element>;
  }
}
