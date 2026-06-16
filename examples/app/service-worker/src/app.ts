// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

const statusEl = document.querySelector<HTMLElement>('[data-status]');

function setStatus(message: string): void {
  if (statusEl) {
    statusEl.textContent = message;
  }
}

async function registerServiceWorker(): Promise<void> {
  if (!('serviceWorker' in navigator)) {
    setStatus('This browser does not support service workers.');
    return;
  }

  const registration = await navigator.serviceWorker.register('./service-worker.js', {
    type: 'module',
  });
  await navigator.serviceWorker.ready;

  setStatus(`Service worker ready (${registration.scope}). Reloading into the stream...`);
  location.reload();
}

registerServiceWorker().catch((error) => {
  setStatus(`Service worker failed: ${error instanceof Error ? error.message : String(error)}`);
});
