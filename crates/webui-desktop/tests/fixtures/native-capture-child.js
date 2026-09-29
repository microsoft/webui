// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

(() => {
  const marker = document.querySelector('#marker');
  const channel = new MessageChannel();
  const parentOrigin = new URL(document.referrer).origin;
  let sent = false;
  function report(mode, frames) {
    if (sent) return;
    sent = true;
    channel.port2.postMessage({
      kind: 'painted', nonce: 'controlled-preview-v1',
      readyId: 'preview-paint-v1',
      marker: getComputedStyle(marker).backgroundColor, mode, frames,
    });
  }
  channel.port2.onmessage = event => {
    if (event.data?.kind !== 'challenge' ||
        event.data?.nonce !== 'controlled-preview-v1') return;
    // Locked desktops can suspend rAF; label fallback explicitly and prove
    // actual paint with two native WK snapshot pixel reads before test capture.
    setTimeout(() => report('timer-fallback', 0), 1500);
    requestAnimationFrame(() => requestAnimationFrame(() => report('two-raf', 2)));
  };
  channel.port2.start();
  window.parent.postMessage(
    {kind: 'offer', nonce: 'controlled-preview-v1'},
    parentOrigin, [channel.port1],
  );
})();
