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
    // Fetch the group as a server resource. The endpoint canonicalizes the
    // case-insensitive slug (e.g. `/groups/work` -> `Work`) and applies the
    // `q` search filter, matching the SSR path, so the display name stays
    // canonical even when the search narrows the list to nothing.
    return api.groups.get(params.group, { q: query.q || undefined }, { signal });
  }
}

CbPageGroup.define('cb-page-group');
