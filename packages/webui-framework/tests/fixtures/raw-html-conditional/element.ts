// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

interface KeyedRawItem {
  id: string;
  label: string;
  rawHtml: string;
  showRaw: boolean;
}

export class TestRawHtml extends WebUIElement {
  @observable expanded = true;
  @observable firstRawHtml = '';
  @observable inlineName = '';
  @observable inlineRawHtml = '';
  @observable name = '';
  @observable rawHtml = '';
  @observable rowsHtml = '';
  @observable secondRawHtml = '';
  @observable styleRule = '';
}

export class TestRawHtmlKeyedRepeat extends WebUIElement {
  @observable items: KeyedRawItem[] = [];

  reverseItems(): void {
    this.items = this.items.map((item) => ({ ...item })).reverse();
  }

  updateItemRaw(id: string): void {
    this.items = this.items.map((item) => (
      item.id === id
        ? {
            ...item,
            rawHtml:
              `<strong class="raw-node raw-first" data-owner="${id}">${id} updated 1</strong>`
              + `<u class="raw-node raw-second" data-owner="${id}">${id} updated 2</u>`,
          }
        : item
    ));
  }

  updateItemRawHtml(id: string, rawHtml: string): void {
    this.items = this.items.map((item) => (
      item.id === id ? { ...item, rawHtml } : { ...item }
    ));
  }

  removeItem(id: string): void {
    this.items = this.items.filter((item) => item.id !== id);
  }
}

TestRawHtml.define('test-raw-html');
TestRawHtmlKeyedRepeat.define('test-raw-html-keyed-repeat');
