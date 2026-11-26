// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '@microsoft/webui-framework';

class GreetingCard extends WebUIElement {
  @observable name = '';
}

GreetingCard.define('greeting-card');
