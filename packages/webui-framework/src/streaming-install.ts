// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { beginStreamingGate } from './lifecycle.js';
import {
  enqueueStreamingSentinel,
  installStreamingTruncationGuard,
} from './streaming-coordinator.js';
import { isStreamingHydrationMode } from './streaming-mode.js';
import { installTemplateDefinitionPreparation } from './template-registry.js';
import { prepareTemplateDefinitions } from './streaming-template-hosts.js';

const SENTINEL_TAG = 'webui-hydrate';
let installed = false;

class WebUiHydrateSentinel extends HTMLElement {
  connectedCallback(): void {
    enqueueStreamingSentinel(this);
  }
}

/**
 * Install the document coordinator when the server emitted streaming mode.
 *
 * The call is idempotent. Non-streaming pages pay for one cached marker query.
 */
export function installStreamingCoordinator(): void {
  if (installed || !isStreamingHydrationMode()) return;
  installed = true;
  installTemplateDefinitionPreparation(prepareTemplateDefinitions);

  if (!customElements.get(SENTINEL_TAG)) {
    customElements.define(SENTINEL_TAG, WebUiHydrateSentinel);
  }
  beginStreamingGate();
  installStreamingTruncationGuard();
}

export function resetStreamingInstallForTests(): void {
  installed = false;
}
