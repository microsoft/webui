// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, attr, WebUIElement } from '@microsoft/webui-framework';
class ContactCard extends WebUIElement {
  @observable email = '';
  @attr firstName = '';
  @attr({ attribute: 'last-name' }) lastName = '';
}
ContactCard.define('contact-card');
