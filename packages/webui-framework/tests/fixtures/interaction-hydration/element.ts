// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  installInteractionHydration,
  isInteractionReplay,
} from '../../../src/interaction-hydration.js';

const root = document.querySelector('#interaction-root');
if (!root) throw new Error('interaction hydration fixture root is missing');

let releaseHydration!: () => void;
const hydrationPending = new Promise<void>((resolve) => {
  releaseHydration = resolve;
});
const state = {
  appClicks: 0,
  documentCaptureReplays: [] as boolean[],
  loadCount: 0,
  replayed: false,
  targetId: '',
};

declare global {
  interface Window {
    interactionFixture: typeof state;
    releaseInteractionHydration(): void;
  }
}

window.interactionFixture = state;
window.releaseInteractionHydration = releaseHydration;

document.addEventListener('click', (event) => {
  const target = event.composedPath()[0];
  const replay = isInteractionReplay(event);
  if (
    target instanceof Element
    && target.id === 'blocked-link'
    && !replay
  ) {
    event.preventDefault();
  }
  state.documentCaptureReplays.push(replay);
}, true);

installInteractionHydration({
  load: async () => {
    state.loadCount++;
    await hydrationPending;
    root.addEventListener('click', (event) => {
      const target = event.composedPath()[0];
      state.appClicks++;
      state.replayed = isInteractionReplay(event);
      state.targetId = target instanceof Element ? target.id : '';
    });
  },
  root,
});
