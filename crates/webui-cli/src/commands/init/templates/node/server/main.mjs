// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { pipeline } from 'node:stream/promises';
import { Protocol } from '@microsoft/webui';
import { listenAddress, openAsset, pageState, requestPath } from './http.mjs';

const protocol = new Protocol(await readFile('dist/protocol.bin'), { plugin: 'webui' });
const address = listenAddress();
const server = createServer({ requestTimeout: 15_000, headersTimeout: 10_000 }, async (request, response) => {
  const head = request.method === 'HEAD';
  function text(status, message, headers = {}) {
    response.writeHead(status, { 'content-type': 'text/plain; charset=utf-8', 'x-content-type-options': 'nosniff', ...headers });
    response.end(head ? undefined : message);
  }
  try {
    if (request.method !== 'GET' && !head) { text(405, 'Method not allowed', { allow: 'GET, HEAD' }); return; }
    const pathname = requestPath(request.url ?? '');
    if (!pathname) { text(404, 'Not found'); return; }
    const state = pageState(pathname);
    if (state) {
      const body = protocol.render(state, { entry: 'index.html', requestPath: pathname });
      response.writeHead(200, {
        'content-type': 'text/html; charset=utf-8', 'content-length': body.length,
        'x-content-type-options': 'nosniff', 'cache-control': 'no-store',
      });
      response.end(head ? undefined : body);
      return;
    }
    const asset = await openAsset(pathname);
    if (!asset) { text(404, 'Not found'); return; }
    response.writeHead(200, asset.headers);
    if (head) { await asset.handle.close(); response.end(); }
    else await pipeline(asset.handle.createReadStream(), response);
  } catch (error) {
    if (error.code === 'ERR_STREAM_PREMATURE_CLOSE' || response.destroyed) return;
    console.error('Request failed:', error);
    if (response.headersSent) response.destroy();
    else text(500, 'Internal server error');
  }
});
server.maxHeadersCount = 100;
server.on('clientError', (_error, socket) => socket.destroy());
server.on('error', error => { console.error('Server failed:', error); process.exitCode = 1; });
server.listen(address.port, address.hostname, () => {
  console.log(`Listening on http://${address.hostname}:${address.port}`);
});
let stopping = false;
function shutdown() {
  if (stopping) return;
  stopping = true;
  if (process.env._WEBUI_APP_CONTROL === 'stdin-v1') process.stdin.destroy();
  server.close(error => { if (error) { console.error(error); process.exitCode = 1; } });
  server.closeIdleConnections();
  const timeout = setTimeout(() => server.closeAllConnections(), 5000);
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
