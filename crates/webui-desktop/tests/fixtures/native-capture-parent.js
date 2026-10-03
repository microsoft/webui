// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

(() => {
  const frame = document.querySelector('#preview');
  const origin = new URL(frame.src).origin;
  const state = {
    loadCount: 0, loadAt: 0, offerAt: 0, readyAt: 0,
    marker: '', frames: -1, mode: '', readyId: '', channelReady: false,
  };
  window.__captureProof = state;
  frame.addEventListener('load', () => {
    state.loadCount += 1;
    state.loadAt = Date.now();
  });
  window.addEventListener('message', event => {
    if (event.origin !== origin || event.source !== frame.contentWindow ||
        event.data?.kind !== 'offer' || event.data?.nonce !== 'controlled-preview-v1' ||
        event.ports.length !== 1) return;
    state.offerAt = Date.now();
    const port = event.ports[0];
    port.onmessage = receipt => {
      if (receipt.data?.kind !== 'painted' || receipt.data?.nonce !== 'controlled-preview-v1' ||
          receipt.data?.readyId !== 'preview-paint-v1' ||
          receipt.data?.marker !== 'rgb(255, 154, 0)') return;
      state.readyAt = Date.now();
      state.marker = receipt.data.marker;
      state.frames = receipt.data.frames;
      state.mode = receipt.data.mode;
      state.readyId = receipt.data.readyId;
      state.channelReady = true;
      port.postMessage({kind: 'ack', nonce: 'controlled-preview-v1'});
    };
    port.start();
    port.postMessage({kind: 'challenge', nonce: 'controlled-preview-v1'});
  });
})();
