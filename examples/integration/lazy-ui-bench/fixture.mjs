// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { once } from 'node:events';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';
import { build, Protocol } from '../../../packages/webui/dist/index.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const require = createRequire(join(root, 'packages', 'webui-test-support', 'package.json'));
const { build: bundle, version: esbuildVersion } = require('esbuild');
export { esbuildVersion };
export const modes = ['eager-local', 'eager-fresh', 'lazy'];

function entry(mode) {
  const source = file => JSON.stringify(join(root, 'packages', 'webui-framework', 'src', file));
  return `
import { collectTemplates } from ${source('template-cache-entry.ts')};
import { WebUIElement, observable } from ${source('core.ts')};
${mode === 'lazy' ? `
import ${source('lazy-entry.ts')};
import { createLazyLoader } from ${JSON.stringify(join(root, 'packages', 'webui-router', 'src', 'lazy-entry.ts'))};
const loader = createLazyLoader({ endpoint: new URL('resources', document.baseURI).href });` : ''}
class BenchOwner extends WebUIElement {
  @observable shown = false;
  @observable value = 'Fresh';
  async open() {
    const start = performance.now();
    ${mode === 'lazy'
      ? "this.setState(await loader.fetchPartial('bench-owner', 'panel', { url: new URL('state', document.baseURI).href }));"
      : mode === 'eager-fresh'
        ? "const response = await fetch(new URL('state', document.baseURI)); if (!response.ok) throw new Error('State request failed'); this.setState(await response.json());"
        : ''}
    this.shown = true;
    this.$flushUpdates();
    return performance.now() - start;
  }
  close() { this.shown = false; this.$flushUpdates(); }
}
const started = performance.now();
BenchOwner.define('bench-owner');
window.bench = {
  activationMs: performance.now() - started,
  open: () => document.querySelector('bench-owner').open(),
  close: () => document.querySelector('bench-owner').close(),
  collect: collectTemplates,
  track() {
    const templates = window.__webui.templates;
    this.observed = new WeakRef(${mode === 'lazy' ? "templates['bench-owner::lazy:panel']" : "templates['bench-owner'].b[0]"});
  },
  retained() { return this.observed.deref() !== undefined; },
};
`;
}

export async function startFixtures(counts, latencyMs) {
  const scratch = await mkdtemp(join(tmpdir(), 'webui-lazy-bench-'));
  const fixtures = new Map();
  const initial = { shown: false, value: 'Fresh' };
  try {
    for (const mode of modes) {
      const javascript = await bundle({
        stdin: { contents: entry(mode), loader: 'ts', resolveDir: root },
        bundle: true, write: false, minify: true, format: 'iife', platform: 'browser',
        target: 'es2022', tsconfig: join(root, 'tsconfig.json'),
        define: { __WEBUI_DEV__: 'false' }, legalComments: 'none',
      });
      const script = Buffer.from(javascript.outputFiles[0].contents);
      for (const count of counts) {
        const base = `/${mode}/${count}/`;
        const html = `<html><head><base href="${base}"><script type="module" src="app.js"></script></head><body><bench-owner></bench-owner></body></html>`;
        const body = Array.from({ length: count }, (_, index) =>
          `<label>Setting ${index}<input value="{{value}}"></label>`).join('');
        const template = `<h1>Shell</h1><if condition="shown"${mode === 'lazy' ? ' lazy id="panel"' : ''}><section>${body}</section></if>`;
        await writeFile(join(scratch, 'index.html'), html);
        await writeFile(join(scratch, 'bench-owner.html'), template);
        await writeFile(join(scratch, 'bench-owner.ts'), entry(mode));
        const built = build({ appDir: scratch, plugin: 'webui' });
        const protocol = new Protocol(built.protocol, { plugin: 'webui' });
        const document = Buffer.from(protocol.render(initial, { entry: 'index.html' }));
        fixtures.set(base, {
          protocol, document, script, gzipDocument: gzipSync(document), gzipScript: gzipSync(script),
          mode, count,
        });
      }
    }
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
  const failures = [];
  const server = createServer((request, response) => {
    void (async () => {
      const url = new URL(request.url, 'http://127.0.0.1');
      const segments = url.pathname.split('/');
      const fixture = fixtures.get(`/${segments[1]}/${segments[2]}/`);
      if (!fixture) { response.writeHead(404).end(); return; }
      const resource = segments[3];
      let bytes, type;
      if (!resource) { bytes = fixture.gzipDocument; type = 'text/html'; }
      else if (resource === 'app.js') { bytes = fixture.gzipScript; type = 'text/javascript'; }
      else {
        const options = {
          inventory: request.headers['x-webui-inventory'] ?? '',
          styleInventory: request.headers['x-webui-style-inventory'] ?? '',
          buildId: request.headers['x-webui-build'] ?? '',
        };
        let json;
        if (resource === 'resources') {
          json = fixture.protocol.renderLazyResources('bench-owner', 'panel', options);
        } else if (resource === 'state') {
          await delay(latencyMs);
          json = fixture.mode === 'lazy'
            ? fixture.protocol.renderLazyPartial({ value: 'Fresh' }, 'bench-owner', 'panel', options)
            : JSON.stringify({ value: 'Fresh' });
        } else { response.writeHead(404).end(); return; }
        bytes = gzipSync(json);
        type = 'application/json';
      }
      response.writeHead(200, {
        'Content-Type': type, 'Content-Encoding': 'gzip',
        'Content-Length': bytes.length, 'Cache-Control': 'no-store',
      });
      response.end(bytes);
    })().catch(error => {
      failures.push(error);
      console.error(error);
      response.writeHead(500).end(String(error));
    });
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return {
    fixtures, origin: `http://127.0.0.1:${server.address().port}`,
    async close() {
      await new Promise((resolve, reject) => {
        server.close(error => error ? reject(error) : resolve());
        server.closeAllConnections();
      });
      assert.deepEqual(failures, [], 'all fixture requests must succeed');
    },
  };
}
