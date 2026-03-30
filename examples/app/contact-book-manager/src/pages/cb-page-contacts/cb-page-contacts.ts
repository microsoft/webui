// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import '#organisms/cb-contact-card/cb-contact-card.js';
import { Contact } from '#api';

export class CbPageContacts extends WebUIElement {
  @observable contacts: Contact[] = [];
}

CbPageContacts.define('cb-page-contacts');
