// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { RouteTransitions } from './capabilities.js';

/** Opt-in same-document transitions without enabling cross-document animation. */
export const viewTransitions: RouteTransitions = {
  supported: () => typeof document.startViewTransition === 'function',
  install() {
    if (!viewTransitions.supported()) return () => {};
    const style = document.createElement('style');
    style.textContent = '@view-transition { navigation: none; }';
    const nonce = window.__webui?.nonce;
    if (nonce) style.nonce = nonce;
    document.head.appendChild(style);
    return () => style.remove();
  },
  startViewTransition: commit => document.startViewTransition(commit),
};
