// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { readFile } from 'node:fs/promises';
import { Readable } from 'node:stream';
import { Protocol } from '@microsoft/webui';
import { listenAddress, openAsset, pageState, requestPath } from './http.mjs';

const protocol = new Protocol(await readFile('dist/protocol.bin'), { plugin: 'webui' });
const address = listenAddress();
const server = Bun.serve({
  ...address,
  idleTimeout: 15,
  maxRequestBodySize: 1024,
  async fetch(request) {
    const head = request.method === 'HEAD';
    const text = (status: number, message: string, headers = {}) => new Response(head ? null : message, {
      status, headers: { 'content-type': 'text/plain; charset=utf-8', 'x-content-type-options': 'nosniff', ...headers },
    });
    try {
      if (request.method !== 'GET' && !head) return text(405, 'Method not allowed', { allow: 'GET, HEAD' });
      const pathname = requestPath(new URL(request.url).pathname);
      if (!pathname) return text(404, 'Not found');
      const state = pageState(pathname);
      if (state) {
        const body = protocol.render(state, { entry: 'index.html', requestPath: pathname });
        return new Response(head ? null : body, {
          headers: {
            'content-type': 'text/html; charset=utf-8', 'content-length': String(body.length),
            'x-content-type-options': 'nosniff', 'cache-control': 'no-store',
          },
        });
      }
      const asset = await openAsset(pathname);
      if (!asset) return text(404, 'Not found');
      if (head) { await asset.handle.close(); return new Response(null, { headers: asset.headers }); }
      return new Response(Readable.toWeb(asset.handle.createReadStream()) as ReadableStream, { headers: asset.headers });
    } catch (error) {
      console.error('Request failed:', error);
      return text(500, 'Internal server error');
    }
  },
});
console.log(`Listening on http://${address.hostname}:${address.port}`);
let stopping = false;
function shutdown() {
  if (stopping) return;
  stopping = true;
  if (process.env._WEBUI_APP_CONTROL === 'stdin-v1') process.stdin.destroy();
  void server.stop(false);
  const timeout = setTimeout(() => { void server.stop(true); }, 5000);
  timeout.unref();
}
process.once('SIGINT', shutdown);
process.once('SIGTERM', shutdown);
if (process.env._WEBUI_APP_CONTROL === 'stdin-v1') {
  process.stdin.once('data', command => {
    if (command[0] !== 83) console.error('Invalid WebUI supervisor command; shutting down');
    shutdown();
  });
  process.stdin.once('end', () => {
    if (!stopping) console.error('WebUI supervisor disconnected; shutting down');
    shutdown();
  });
  process.stdin.once('error', error => {
    console.error('WebUI supervisor control failed:', error);
    shutdown();
  });
  process.stdin.resume();
}
