// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import '#atoms/cb-empty-state/cb-empty-state.js';
import '#organisms/cb-contact-card/cb-contact-card.js';
import type { Contact } from '#api';

export class CbPageGroup extends WebUIElement {
  @observable groupName = '';
  @observable contacts: Contact[] = [];
}

CbPageGroup.define('cb-page-group');
