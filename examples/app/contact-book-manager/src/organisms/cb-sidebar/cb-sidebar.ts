// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

export class CbSidebar extends WebUIElement {
  @attr page = 'dashboard';
  @attr activeGroup = 'all';
  @attr totalContacts = '0';
  @attr totalFavorites = '0';
  @observable groups: string[] = [];
}

CbSidebar.define('cb-sidebar');
