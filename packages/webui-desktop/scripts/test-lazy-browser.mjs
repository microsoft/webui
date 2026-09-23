// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Run after `pnpm build`. Uses the workspace's existing Playwright installation.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';
import { build } from 'esbuild';
import { chromium, firefox, webkit } from '@playwright/test';
import { defaultLimits } from '../dist/index.js';

const root = new URL('../', import.meta.url);
const result = await build({
  entryPoints: [fileURLToPath(new URL('../../crates/webui-desktop/tests/fixtures/typed-ipc/ts/ipc.ts', root))],
  outdir: fileURLToPath(new URL('dist-lazy-test/', root)),
  write: false, bundle: true, splitting: true, format: 'esm', platform: 'browser',
  target: 'es2022', minify: true, metafile: true,
  alias: {
    '@microsoft/webui-desktop': fileURLToPath(new URL('src/index.ts', root)),
  },
});
const files = new Map(result.outputFiles.map(file => [`/${basename(file.path)}`, file.contents]));
const deferred = [...files.keys()].filter(path => path !== '/ipc.js');
assert.equal(deferred.length, 1);
assert(files.get('/ipc.js').length < 512);
for (const [path, bytes] of files) console.log(`${path}: ${bytes.length} bytes, ${gzipSync(bytes).length} gzip`);

let requests = [];
let failChunk = false;
const server = createServer((request, response) => {
  if (request.url === '/') {
    response.setHeader('Content-Type', 'text/html');
    response.end('<!doctype html><title>Lazy IPC module boundary</title>');
    return;
  }
  requests.push(request.url);
  const bytes = files.get(request.url);
  response.setHeader('Cache-Control', 'no-store');
  response.setHeader('Content-Type', 'text/javascript');
  response.statusCode = bytes && !(failChunk && deferred.includes(request.url)) ? 200 : 503;
  response.end(response.statusCode === 200 ? bytes : 'Unavailable');
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
try {
  for (const engine of [chromium, firefox, webkit]) {
    const browser = await engine.launch();
    try {
      const page = await browser.newPage();
      await page.goto(origin);
      requests = [];
      const exported = await page.evaluate(async () => {
        globalThis.bindings = await import('/ipc.js');
        return Object.keys(globalThis.bindings).sort();
      });
      assert.deepEqual(exported, ['connectDesktop', 'schemaHash']);
      assert.deepEqual(requests.filter(path => files.has(path)), ['/ipc.js']);
      const connections = await page.evaluate(async limits => {
        let starts = 0;
        const transports = [0, 1].map(() => ({
          async start(hello, receiver) {
            starts++;
            this.hello = hello;
            this.receiver = receiver;
            return { generation: '1', token: 'a'.repeat(32), limits };
          },
          async send() { throw new Error('unexpected send'); },
          close() { this.closed = true; },
        }));
        const [first, second] = await Promise.all(transports.map(transport => globalThis.bindings.connectDesktop(transport)));
        first.close();
        const reason = await first.closed;
        const independent = !transports[1].closed && transports[0].receiver !== transports[1].receiver;
        second.close();
        return { starts, independent, code: reason.code, helloKeys: Object.keys(transports[0].hello).sort() };
      }, defaultLimits);
      assert.deepEqual(connections, {
        starts: 2, independent: true, code: 'closed',
        helloKeys: ['contractMajor', 'contractName', 'schemaHash', 'wireVersion'],
      });
      assert.deepEqual(requests.filter(path => files.has(path)), ['/ipc.js', ...deferred]);
      await page.close();

      // A new document with a failed chunk cannot activate a transport, and a
      // later explicit connect does not silently retry its rejected module load.
      const failed = await browser.newPage();
      await failed.goto(origin);
      requests = [];
      failChunk = true;
      const rejection = await failed.evaluate(async () => {
        globalThis.bindings = await import('/ipc.js');
        globalThis.starts = 0;
        globalThis.transport = { start() { globalThis.starts++; }, send() {}, close() {} };
        const results = await Promise.allSettled([
          globalThis.bindings.connectDesktop(globalThis.transport),
          globalThis.bindings.connectDesktop(globalThis.transport),
        ]);
        return { statuses: results.map(result => result.status), starts: globalThis.starts };
      });
      assert.deepEqual(rejection, { statuses: ['rejected', 'rejected'], starts: 0 });
      const requestCount = requests.filter(path => files.has(path)).length;
      failChunk = false;
      assert.equal(await failed.evaluate(async () => {
        try { await globalThis.bindings.connectDesktop(globalThis.transport); return 'resolved'; }
        catch { return 'rejected'; }
      }), 'rejected');
      assert.equal(requests.filter(path => files.has(path)).length, requestCount);
      console.log(`${engine.name()}: import isolation, concurrent independent connects, chunk failure and no retry passed`);
    } finally {
      failChunk = false;
      await browser.close();
    }
  }
} finally {
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
}
