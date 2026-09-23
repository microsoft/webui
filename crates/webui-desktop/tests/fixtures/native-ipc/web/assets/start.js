// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Diagnostic-only custom-protocol endpoint; never substitutes for typed IPC.
function diagnostic(message) {
  return fetch('/fixture-diagnostic', { method: 'POST', body: message });
}
window.addEventListener('pageshow', event => {
  if (event.persisted) {
    void resume().catch(error => diagnostic(`failure restored code=${error?.code} message=${error?.message} stack=${error?.stack ?? error}`));
  }
});
async function resume() {
  await diagnostic('pageshow persisted: explicitly reconnecting');
  const { run } = await import('/fixture.js');
  await run(true);
}
await diagnostic(`script-start visibility=${document.visibilityState} bootstrap=${!!window.__webuiDesktopIpcV2} nativeHandler=${!!(window.webkit?.messageHandlers?.webuiDesktopIpc || window.chrome?.webview)}`);
window.addEventListener('error', event => { void diagnostic(`failure error ${event.message}`); });
window.addEventListener('unhandledrejection', event => { void diagnostic(`failure rejection ${event.reason}`); });
try {
  // Module loads use the browser resource path, not the legacy fetch bridge.
  // There is deliberately no on-disk fallback in source or packaged assets.
  const { servedByRuntime } = await import('/fixture-runtime.js');
  if (servedByRuntime !== true) throw new Error('Native runtime resource interception failed');
  await diagnostic('runtime-resource-probe passed');
  const { run } = await import('/fixture.js');
  await run();
} catch (error) {
  await diagnostic(`failure code=${error?.code} message=${error?.message} stack=${error?.stack ?? error}`);
}
