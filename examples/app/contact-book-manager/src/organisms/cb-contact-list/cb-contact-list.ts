// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

// Child components used in cb-contact-list.html
import '#atoms/cb-empty-state/cb-empty-state.js';
import '#organisms/cb-contact-card/cb-contact-card.js';

interface Contact {
  id: string;
  firstName: string;
  lastName: string;
  email: string;
  phone: string;
  company: string;
  group: string;
  favorite: boolean;
  initials: string;
  avatarColor: string;
  notes: string;
  address: string;
}

export class CbContactList extends WebUIElement {
  @observable contacts!: Contact[];
}

CbContactList.define('cb-contact-list');
