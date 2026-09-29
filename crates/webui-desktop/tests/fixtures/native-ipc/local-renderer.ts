// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createNativeDesktopTransport } from '@microsoft/webui-desktop/native';
import { connectDesktop } from './generated/ts/ipc-runtime.js';

type FixtureWindow = Window & {
  webkit?: { messageHandlers?: {
    webuiDesktopIpc?: { postMessage(message: unknown): unknown };
    webuiDesktopIpcData?: { postMessage(message: unknown): unknown };
  } };
  chrome?: { webview?: { postMessage(message: unknown): void } };
};
const nativeWindow = window as FixtureWindow;
const previewHost = `p-alpha.preview.localhost:${location.port}`;

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

async function preview(): Promise<void> {
  if (location.pathname === '/preview') {
    location.replace('/preview/deep?lease=alpha');
    return;
  }
  assert(location.pathname === '/preview/deep', 'preview nested route not loaded');
  const mainOrigin = `http://127.0.0.1:${location.port}`;
  const result = new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('preview credential transfer timed out')), 10000);
    const onMessage = async (event: MessageEvent) => {
      if (event.source !== parent || event.origin !== mainOrigin || event.data?.kind !== 'preview-test') return;
      window.removeEventListener('message', onMessage);
      try {
        assert(event.ports.length === 1, 'preview channel missing');
        const port = event.ports[0];
        const { generation, token } = event.data;
        assert(typeof generation === 'string' && typeof token === 'string', 'test credential missing');
        // Only this cross-origin, sandboxed child sends these messages. The
        // parent never invokes its own bridge on the child's behalf.
        const control = { kind: 'disconnect', generation, token };
        const data = { kind: 'ipcData', version: 1, callId: '1', generation, token,
          operation: 'receive', offset: 0, maxBytes: 64 };
        if (nativeWindow.webkit?.messageHandlers?.webuiDesktopIpc) {
          const handlers = nativeWindow.webkit.messageHandlers;
          assert(handlers.webuiDesktopIpcData, 'WK child data handler unavailable');
          for (const [handler, message] of [
            [handlers.webuiDesktopIpc, control],
            [handlers.webuiDesktopIpcData, data],
          ] as const) {
            let rejected = false;
            try { await handler!.postMessage(message); } catch { rejected = true; }
            assert(rejected, 'WK accepted a child IPC message with a current credential');
          }
        } else if (nativeWindow.chrome?.webview) {
          // WebView2 postMessage has no reply; the following parent RPC proves
          // the forged disconnect did not retire its session.
          nativeWindow.chrome.webview.postMessage(control);
          nativeWindow.chrome.webview.postMessage(data);
        } else {
          throw new Error('no native child message handler');
        }
        // Empty same-origin POST; neither credential nor data is sent to HTTP.
        const receipt = await fetch('/preview/receipt', { method: 'POST', body: '' });
        assert(receipt.ok, 'preview receipt rejected');
        port.postMessage('denied');
        clearTimeout(timer);
        resolve();
      } catch (error) {
        event.ports[0]?.postMessage('failed');
        clearTimeout(timer);
        reject(error);
      }
    };
    window.addEventListener('message', onMessage);
  });
  parent.postMessage({ kind: 'preview-ready' }, mainOrigin);
  await result;
}

async function verifyPreview(credentials: { generation: string; token: string }): Promise<void> {
  // Wrong lease uses the same listener port but has no exact-origin grant.
  const wrong = document.createElement('iframe');
  wrong.hidden = true;
  wrong.sandbox.add('allow-scripts');
  wrong.src = `http://p-beta.preview.localhost:${location.port}/preview?lease=beta`;
  document.body.append(wrong);

  const child = document.createElement('iframe');
  child.sandbox.add('allow-scripts', 'allow-same-origin', 'allow-forms', 'allow-downloads');
  child.src = `http://${previewHost}/preview?lease=alpha`;
  document.body.append(child);
  try {
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('exact preview did not report')), 12000);
      const onMessage = (event: MessageEvent) => {
        if (event.source !== child.contentWindow || event.origin !== `http://${previewHost}` ||
            event.data?.kind !== 'preview-ready') return;
        window.removeEventListener('message', onMessage);
        const channel = new MessageChannel();
        channel.port1.onmessage = reply => {
          clearTimeout(timer);
          channel.port1.close();
          if (reply.data === 'denied') resolve();
          else reject(new Error('child IPC denial failed'));
        };
        child.contentWindow!.postMessage({ kind: 'preview-test', ...credentials },
          `http://${previewHost}`, [channel.port2]);
      };
      window.addEventListener('message', onMessage);
    });
  } finally {
    child.remove();
    wrong.remove();
  }
}

async function run(): Promise<void> {
  if (window !== window.top) return preview();
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
  if (nativeWindow.webkit?.messageHandlers?.webuiDesktopIpc) {
    const child = document.createElement('iframe');
    document.body.append(child);
    const channel = (child.contentWindow as FixtureWindow | null)?.webkit?.messageHandlers?.webuiDesktopIpc;
    if (!channel) throw new Error('WK child handler unavailable for trust regression');
    try {
      await channel.postMessage({
        kind: 'disconnect', generation: credentials.generation, token: credentials.token,
      });
    } catch { /* A non-main frame is denied by native frameInfo. */ }
    child.remove();
  }
  if (location.pathname === '/' && document.documentElement.dataset.preview === 'true') {
    await verifyPreview(credentials);
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
  console.error('Native local fixture failed', error);
  // No error string or native credential ever crosses the HTTP boundary.
  await fetch('/fail', { method: 'POST' });
});
