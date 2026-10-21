// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createDeferredLoader } from '@microsoft/webui-router/deferred.js';
import { deferredNdjson } from '@microsoft/webui-router/deferred-ndjson.js';

const endpoint = new URL('_webui/deferred', document.baseURI).href;

export const fragments = createDeferredLoader({ endpoint, transport: deferredNdjson });

export function stateUrl(owner: string, id: 'activity' | 'settings', slowData: boolean): string {
  const query = new URLSearchParams({ owner, id });
  if (slowData) query.set('slow', '1');
  return `${endpoint}/state?${query}`;
}
