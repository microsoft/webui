// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import '../asset-badge/asset-badge.js';

export class LazyPanel extends WebUIElement {
  @observable status = 'Loading';
  @observable heading = 'Loading panel data';
  @observable message = 'The component class is fetching its own JSON state.';
  @observable hasDetails = false;
  @observable details = '';
}

LazyPanel.define('lazy-panel');
