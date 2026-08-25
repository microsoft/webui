// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  installInteractionHydration,
  isInteractionReplay,
} from '../../../src/interaction-hydration.js';

interface InteractionFixtureState {
  appClicks: number;
  documentCaptureReplays: boolean[];
  loadCount: number;
  replayed: boolean;
  targetId: string;
}

declare global {
  interface Window {
    interactionFixture: InteractionFixtureState;
    releaseInteractionHydration(): void;
  }
}

const root = document.querySelector('#interaction-root');
if (!root) throw new Error('interaction hydration fixture root is missing');

let releaseHydration!: () => void;
const hydrationPending = new Promise<void>((resolve) => {
  releaseHydration = resolve;
});
const state: InteractionFixtureState = {
  appClicks: 0,
  documentCaptureReplays: [],
  loadCount: 0,
  replayed: false,
  targetId: '',
};
window.interactionFixture = state;
window.releaseInteractionHydration = releaseHydration;

document.addEventListener('click', (event) => {
  const target = event.composedPath()[0];
  if (
    target instanceof Element
    && target.id === 'blocked-link'
    && !isInteractionReplay(event)
  ) {
    event.preventDefault();
  }
  state.documentCaptureReplays.push(isInteractionReplay(event));
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
