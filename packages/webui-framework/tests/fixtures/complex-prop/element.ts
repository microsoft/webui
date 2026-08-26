// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

export class TestItemList extends WebUIElement {
  @observable items: Array<{ name: string }> = [];
}

/** Child component with a conditional block driven by a complex :data property. */
export class TestCondChild extends WebUIElement {
  @observable data: { showHeader?: boolean; label?: string } = {};
}

export class TestPropertyChild extends WebUIElement {
  @observable prop: { label?: string } = {};
}

export class TestItemHost extends WebUIElement {
  @observable sourceItems: Array<{ name: string }> = [
    { name: 'Alpha' },
    { name: 'Beta' },
    { name: 'Gamma' },
  ];

  @observable condData = { showHeader: true, label: 'Hello' };
  @observable passedProperty = { label: 'Initial property' };
  @observable delayedItems: Array<{ name: string }> = [
    { name: 'Alpha' },
    { name: 'Beta' },
    { name: 'Gamma' },
  ];

  replaceItems(): void {
    this.sourceItems = [{ name: 'One' }, { name: 'Two' }];
  }

  clearItems(): void {
    this.sourceItems = [];
  }

  hideCondHeader(): void {
    this.condData = { ...this.condData, showHeader: false };
  }

}

class TestDelayedPropChild extends WebUIElement {
  @observable items: Array<{ name: string }> = [];
}

const defineTestItemHost = (): void => TestItemHost.define('test-item-host');
const defineTestItemList = (): void => TestItemList.define('test-item-list');
const defineTestCondChild = (): void => TestCondChild.define('test-cond-child');
const defineTestPropertyChild = (): void =>
  TestPropertyChild.define('test-property-child');

const hydratedHost = document.querySelector('#host') as TestItemHost | null;
if (!hydratedHost) {
  throw new Error('Complex-property test host is unavailable.');
}

const definitionOrder = new URL(window.location.href).searchParams.get(
  'definitionOrder',
);
if (definitionOrder === 'parent-first') {
  defineTestItemHost();
  defineTestItemList();
  defineTestCondChild();
  defineTestPropertyChild();
} else if (definitionOrder === 'detached-defined-child') {
  hydratedHost.remove();
  defineTestItemList();
  defineTestCondChild();
  defineTestPropertyChild();
  defineTestItemHost();
  document.body.append(hydratedHost);
} else {
  defineTestItemList();
  defineTestCondChild();
  defineTestPropertyChild();
  defineTestItemHost();
}

hydratedHost.delayedItems = [
  { name: 'Late Alpha' },
  { name: 'Late Beta' },
];

queueMicrotask(() => {
  TestDelayedPropChild.define('test-delayed-prop-child');
});
