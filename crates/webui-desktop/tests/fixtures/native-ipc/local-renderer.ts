// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createNativeDesktopTransport } from '@microsoft/webui-desktop/native';
import { connectDesktop } from './generated/ts/ipc-runtime.js';

async function run(): Promise<void> {
  if (!window.__webuiDesktopIpcV2) throw new Error('local native bootstrap absent');
  const transport = createNativeDesktopTransport();
  let credentials: { generation: string; token: string } | undefined;
  const start = transport.start.bind(transport);
  transport.start = async (...args) => {
    const session = await start(...args);
    if (session.nativeCarrierVersion !== 1) throw new Error('native carrier was not admitted');
    credentials = session;
    return session;
  };
  const connection = await connectDesktop(transport);
  if (!credentials) throw new Error('admission did not return credentials');
  // WK registers a script handler for every frame, but only actual frameInfo
  // may authorize controls. An inherited-origin about:blank child with a real
  // current credential must not disconnect the owning top-level document.
  if (window.webkit?.messageHandlers?.webuiDesktopIpc) {
    const child = document.createElement('iframe');
    document.body.append(child);
    const channel = child.contentWindow?.webkit?.messageHandlers?.webuiDesktopIpc;
    if (!channel) throw new Error('WK child handler unavailable for trust regression');
    try {
      await channel.postMessage({
        kind: 'disconnect', generation: credentials.generation, token: credentials.token,
      });
    } catch { /* A non-main frame is denied by native frameInfo. */ }
    child.remove();
  }
  const image = new Uint8Array(262144);
  for (let i = 0; i < image.length; i++) image[i] = (i * 31 + 7) % 256;
  await connection.host.save({ id: 18446744073709551615n, image, phase: location.pathname });
  connection.close();
  if (location.pathname === '/') {
    location.assign('/next');
    return;
  }
  if (location.pathname !== '/next') throw new Error('unexpected document path');
  const response = await fetch('/pass', { method: 'POST' });
  if (!response.ok) throw new Error(`pass report rejected: ${response.status}`);
}

run().catch(async error => {
  await fetch('/fail', { method: 'POST', body: String(error) });
});
