// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import '#atoms/cb-empty-state/cb-empty-state.js';
import '#organisms/cb-contact-card/cb-contact-card.js';
import { Contact } from '#api';

export class CbPageFavorites extends WebUIElement {
  @observable contacts: Contact[] = [];
}

CbPageFavorites.define('cb-page-favorites');
