// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createNativeDesktopTransport } from '@microsoft/webui-desktop/native';
import { connectDesktop } from './native-ipc/generated/ts/ipc-runtime.js';

type TestWindow = Window & {
  webkit?: { messageHandlers?: {
    webuiDesktopIpc?: unknown;
    webuiDesktopIpcData?: unknown;
  } };
};
let phase = 'entry';

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

async function run(): Promise<void> {
  phase = 'bootstrap';
  assert(window === window.top, 'unexpected subframe module load');
  assert(typeof (window as TestWindow).webkit?.messageHandlers?.webuiDesktopIpc === 'undefined',
    'raw native handler leaked into main world');
  assert(typeof (window as TestWindow).webkit?.messageHandlers?.webuiDesktopIpcData === 'undefined',
    'raw native data handler leaked into main world');
  assert(window.__webuiDesktopIpcV2, 'Linux top-frame bootstrap absent');
  const transport = createNativeDesktopTransport();
  let credentials: { generation: string; token: string } | undefined;
  const start = transport.start.bind(transport);
  transport.start = async (...args) => {
    const session = await start(...args);
    assert(session.nativeCarrierVersion === 1, 'Linux native carrier not admitted');
    credentials = session;
    return session;
  };
  const connection = await connectDesktop(transport);
  assert(credentials, 'Linux IPC session credential absent');
  // Production denies subframe commits. A separate real GTK trust probe
  // loads an opaque child using this same generated isolated mediator.
  phase = 'generated-rpc';
  const image = new Uint8Array(262144);
  for (let i = 0; i < image.length; i++) image[i] = (i * 31 + 7) % 256;
  await connection.host.save({ id: 18446744073709551615n, image, phase: location.pathname });
  connection.close();
  if (location.pathname === '/') {
    phase = 'navigation';
    location.assign('/next');
    return;
  }
  assert(location.pathname === '/next', 'unexpected Linux IPC document');
  phase = 'report';
  const response = await fetch('/pass', { method: 'POST', body: '' });
  assert(response.ok, `Linux IPC pass report failed: ${response.status}`);
}

run().catch(async error => {
  console.error('Linux native IPC fixture failed', error);
  // Report only a fixed phase and machine-readable category, never native
  // credentials or arbitrary error text over the HTTP listener.
  const code = error && typeof error === 'object' && typeof error.code === 'string' &&
    /^[a-z-]{1,32}$/.test(error.code) ? error.code : 'unexpected';
  await fetch(`/fail?step=${phase}&code=${code}`, { method: 'POST', body: '' });
});
