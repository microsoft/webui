// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '../../../src/index.js';
import { findOrCreateRouteElement } from '../../../../webui-router/src/chain.js';

export class TestOutletShell extends WebUIElement {
  @observable rawHtml = '';
  @observable showPrimary = true;
  @observable prefix = '';
  @observable suffix = '';
  @observable emptyPrefix = '';
  @observable emptySuffix = '';
  @observable fragmentPrefix = '';
  @observable fragmentSuffix = '';

  onUpdate(): void {
    this.prefix = 'next-left';
    this.suffix = 'next-right';
    this.emptyPrefix = 'next-empty-left';
    this.emptySuffix = 'next-empty-right';
    this.fragmentPrefix = 'next-fragment-left';
    this.fragmentSuffix = 'next-fragment-right';
  }

  onAddRoute(): void {
    const parent = this.parentElement;
    if (!parent) throw new Error('The outlet fixture must be mounted in its route.');
    findOrCreateRouteElement(
      { component: this.localName, path: '/', params: {}, el: parent, compEl: this },
      { component: 'test-outlet-leaf', path: 'added', params: {} },
    );
  }

  onTogglePrimary(): void {
    this.showPrimary = !this.showPrimary;
  }
}
TestOutletShell.define('test-outlet-shell');

export class TestOutletPlain extends TestOutletShell {}
TestOutletPlain.define('test-outlet-plain');

export class TestOutletUnknown extends TestOutletShell {
  protected override $shouldApplySSRBootstrapState(): boolean {
    return false;
  }

  override onUpdate(): void {
    this.suffix = 'next-right';
  }
}
TestOutletUnknown.define('test-outlet-unknown');

export class TestOutletLeaf extends WebUIElement {}
TestOutletLeaf.define('test-outlet-leaf');
