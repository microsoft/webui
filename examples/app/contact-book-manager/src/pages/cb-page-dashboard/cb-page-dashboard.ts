// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

// Child components used in cb-page-dashboard.html
import '#organisms/cb-contact-card/cb-contact-card.js';
import { Contact } from '#api';

export class CbPageDashboard extends WebUIElement {
  @observable totalContacts = '0';
  @observable totalFavorites = '0';
  @observable totalGroups = '0';
  @observable recentContacts: Contact[] = [];
}

CbPageDashboard.define('cb-page-dashboard');
