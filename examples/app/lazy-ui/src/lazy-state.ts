// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createLazyLoader } from '@microsoft/webui-router/lazy.js';
import { lazyNdjson } from '@microsoft/webui-router/lazy-ndjson.js';

const endpoint = new URL('_webui/lazy', document.baseURI).href;

export const fragments = createLazyLoader({ endpoint, transport: lazyNdjson });

export function stateUrl(owner: string, id: 'activity' | 'settings', slowData: boolean): string {
  const query = new URLSearchParams({ owner, id });
  if (slowData) query.set('slow', '1');
  return `${endpoint}/state?${query}`;
}
