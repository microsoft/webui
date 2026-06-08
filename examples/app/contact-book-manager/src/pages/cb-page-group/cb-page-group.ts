// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import type { RouteLoaderContext } from '@microsoft/webui-router';
import { api, type Contact } from '#api';
import '#organisms/cb-contact-list/cb-contact-list.js';

export class CbPageGroup extends WebUIElement {
  @observable groupName = '';
  @observable contacts: Contact[] = [];

  static ssrLoader = true;

  static async loader(
    { params, query, signal }: RouteLoaderContext,
  ): Promise<{ contacts: Contact[]; groupName: string }> {
    const contacts = await api.contacts.list(
      { q: query.q || undefined, group: params.group },
      { signal },
    );
    return { contacts, groupName: params.group };
  }
}

CbPageGroup.define('cb-page-group');
