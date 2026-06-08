// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import type { RouteLoaderContext } from '@microsoft/webui-router';
import { api, type Contact } from '#api';
import '#organisms/cb-contact-list/cb-contact-list.js';

export class CbPageFavorites extends WebUIElement {
  @observable contacts: Contact[] = [];

  static ssrLoader = true;

  static async loader({ query, signal }: RouteLoaderContext): Promise<{ contacts: Contact[] }> {
    const contacts = await api.contacts.list(
      { q: query.q || undefined, favorites: true },
      { signal },
    );
    return { contacts };
  }
}

CbPageFavorites.define('cb-page-favorites');
