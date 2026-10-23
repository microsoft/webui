// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createLazyLoader } from '@microsoft/webui-delivery/lazy.js';
import { ndjson } from '@microsoft/webui-delivery/resources.js';

const endpoint = new URL('_webui/lazy', document.baseURI).href;

export const fragments = createLazyLoader({ endpoint, transport: ndjson() });

export function stateUrl(owner: string, id: 'activity' | 'settings', slowData: boolean): string {
  const query = new URLSearchParams({ owner, id });
  if (slowData) query.set('slow', '1');
  return `${endpoint}/state?${query}`;
}
