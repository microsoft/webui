// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import { observable, WebUIElement } from '../../../src/index.js';

startRuntime();

export class DetailsFixture extends WebUIElement {
  @observable showClient = false;
}

export class DetailsPanel extends WebUIElement {
  @observable details: string | number | boolean | null | undefined = '';
  @observable nonemptyDetails = '';
  @observable first = '';
  @observable middle = '';
  @observable last = '';
  @observable showMiddle = true;
  @observable items: { id?: number; value: string }[] = [];
}

DetailsFixture.define('details-fixture');
DetailsPanel.define('details-panel');
