// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '../../../src/index.js';

export class TestLinkHost extends WebUIElement {
  spawnChild(): void {
    const root = this.shadowRoot ?? this;
    const slot = root.querySelector('.slot');
    if (!(slot instanceof HTMLDivElement)) {
      throw new Error('Missing .slot container');
    }
    if (!slot.querySelector('test-link-child')) {
      slot.appendChild(document.createElement('test-link-child'));
    }
  }
}

export class TestLinkChild extends WebUIElement {
  @observable message = 'initial';
  hydratedCount = 0;
  hydratedHadContent = false;

  protected override hydratedCallback(): void {
    this.hydratedCount += 1;
    this.hydratedHadContent =
      this.shadowRoot?.querySelector('.child-label') !== null;
  }
}
export class TestLinkDynamicChild extends WebUIElement {
  href = 'child.css';
  media = 'not all';
  rel = 'stylesheet';
}
export class TestLinkImportChild extends WebUIElement {}
export class TestLinkEventChild extends WebUIElement {
  styleLoads = 0;

  onStylesheetLoad(): void {
    this.styleLoads += 1;
  }
}

export class TestStyleBlockChild extends WebUIElement {}

export class TestLifecycleStyleChild extends WebUIElement {
  protected override hydratedCallback(): void {
    if (!this.hasAttribute('data-add-style')) return;
    const style = document.createElement('style');
    style.textContent = '.child-label { color: rgb(255, 0, 0); }';
    this.shadowRoot?.append(style);
  }
}

TestLinkHost.define('test-link-host');
TestLinkChild.define('test-link-child');
TestLinkDynamicChild.define('test-link-dynamic-child');
TestLinkImportChild.define('test-link-import-child');
TestLinkEventChild.define('test-link-event-child');
TestStyleBlockChild.define('test-style-block-child');
TestLifecycleStyleChild.define('test-lifecycle-style-child');
