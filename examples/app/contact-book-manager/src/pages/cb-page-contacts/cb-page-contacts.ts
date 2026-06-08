// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import type { RouteLoaderContext } from '@microsoft/webui-router';
import { api, type Contact } from '#api';
import '#organisms/cb-contact-list/cb-contact-list.js';

export class CbPageContacts extends WebUIElement {
  @observable contacts: Contact[] = [];

  // Run on the initial SSR navigation too, so a direct load of /contacts?q=...
  // renders the filtered list rather than the full server-rendered one.
  static ssrLoader = true;

  static async loader({ query, signal }: RouteLoaderContext): Promise<{ contacts: Contact[] }> {
    const contacts = await api.contacts.list({ q: query.q || undefined }, { signal });
    return { contacts };
  }
}

CbPageContacts.define('cb-page-contacts');
