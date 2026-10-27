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

function entry(mode, interactive = false) {
  const source = file => JSON.stringify(join(root, 'packages', 'webui-framework', 'src', file));
  return `
import { collectTemplates } from ${source('template-cache-entry.ts')};
import { startRuntime } from ${source('runtime.ts')};
import { WebUIElement, observable } from ${source('index.ts')};
${interactive ? `import ${source('styles-entry.ts')};` : ''}
${mode === 'lazy' ? `import ${source('lazy-entry.ts')};` : ''}
startRuntime();
async function fetchState() {
  const response = await fetch(new URL('state', document.baseURI));
  if (!response.ok) throw new Error('State request failed');
  return response.json();
}
class BenchOwner extends WebUIElement {
  @observable shown = false;
  @observable value = 'Fresh';
  ${interactive ? `
  @observable loading = false;
  @observable status = 'Ready';
  @observable shellCount = 0;
  @observable broadcast = false;
  @observable editedValue = '';
  async onOpen(event) {
    const action = window.uiProbe.begin(event, this.loading);
    this.loading = true;
    this.status = 'Loading';
    await this.open();
    this.loading = false;
    this.status = 'Ready';
    this.$flushUpdates();
    window.uiProbe.finish(action);
  }
  onShell(event) {
    const action = window.uiProbe.begin(event, this.loading);
    this.shellCount++;
    queueMicrotask(() => window.uiProbe.finish(action));
  }
  onKeydown(event) {
    if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
      this.inputAction = window.uiProbe.begin(event, this.loading);
    }
  }
  onInput(event) {
    this.editedValue = event.currentTarget.value;
    if (this.broadcast) this.value = this.editedValue;
    const action = this.inputAction;
    this.inputAction = undefined;
    if (action) queueMicrotask(() => window.uiProbe.finish(action));
  }
  changeBroadcast(event) { this.broadcast = event.currentTarget.checked; }
  onClose(event) {
    const action = window.uiProbe.begin(event, this.loading);
    this.close();
    this.status = 'Closed';
    this.$flushUpdates();
    window.uiProbe.finish(action);
  }` : ''}
  async open() {
    const start = performance.now();
    ${mode === 'lazy'
      ? "const [state] = await Promise.all([fetchState(), this.$prefetch()]); this.setState(state); this.shown = true; await this.$rendered();"
      : mode === 'eager-fresh'
        ? "this.setState(await fetchState()); this.shown = true; this.$flushUpdates();"
        : 'this.shown = true; this.$flushUpdates();'}
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
    this.observed = new WeakRef(${mode === 'lazy' ? "templates['bench-owner::lazy:@0']" : "templates['bench-owner'].b[0]"});
  },
  retained() { return this.observed.deref() !== undefined; },
};
`;
}

export async function startFixtures(counts, latencyMs, interactive = false) {
  const scratch = await mkdtemp(join(tmpdir(), 'webui-lazy-bench-'));
  const fixtures = new Map();
  const initial = { shown: false, value: 'Fresh' };
  if (interactive) Object.assign(initial, {
    loading: false, status: 'Ready', shellCount: 0, broadcast: false, editedValue: '',
  });
  try {
    if (interactive) await writeFile(join(scratch, 'bench-owner.css'), `
      :host { display: block; max-width: 1080px; margin: auto; font: 14px system-ui; }
      .toolbar { display: flex; align-items: center; gap: 12px; }
      .edit-preview { display: inline-block; width: 180px; overflow: hidden; white-space: nowrap; }
      section { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px 16px; max-height: 640px; overflow: auto; }
      section label { display: flex; flex-direction: column; }
      section input { box-sizing: border-box; width: 100%; min-width: 0; }
    `);
    for (const mode of modes) {
      const javascript = await bundle({
        stdin: { contents: entry(mode, interactive), loader: 'ts', resolveDir: root },
        bundle: true, write: false, minify: true, format: 'iife', platform: 'browser',
        target: 'es2022', tsconfig: join(root, 'tsconfig.json'),
        define: { __WEBUI_DEV__: 'false' }, legalComments: 'none',
      });
      const script = Buffer.from(javascript.outputFiles[0].contents);
      for (const count of counts) {
        const base = `/${mode}/${count}/`;
        const html = `<html><head><base href="${base}"><script type="module" src="app.js"></script></head><body><bench-owner></bench-owner></body></html>`;
        const body = Array.from({ length: count }, (_, index) =>
          `<label>Setting ${index}<input value="{{value}}"${interactive ? ' @keydown="{onKeydown(e)}" @input="{onInput(e)}"' : ''}></label>`).join('');
        const toolbar = interactive
          ? '<div class="toolbar"><button @click="{onOpen(e)}" ?disabled="{{loading}}">Open panel</button><button @click="{onShell(e)}">Shell click</button><button @click="{onClose(e)}">Close panel</button><output class="shell-count">{{shellCount}}</output><label><input type="checkbox" ?checked="{{broadcast}}" @change="{changeBroadcast(e)}">Broadcast edits</label><output class="edit-preview">{{editedValue}}</output></div><p role="status">{{status}}</p>'
          : '';
        const template = `<h1>Shell</h1>${toolbar}<if condition="shown"${mode === 'lazy' ? ' lazy="demand"' : ''}><section>${body}</section></if>`;
        await writeFile(join(scratch, 'index.html'), html);
        await writeFile(join(scratch, 'bench-owner.html'), template);
        await writeFile(join(scratch, 'bench-owner.ts'), entry(mode, interactive));
        const built = build({ appDir: scratch, plugin: 'webui', ...(interactive ? { css: 'style' } : {}) });
        const protocol = new Protocol(built.protocol, { plugin: 'webui' });
        const document = Buffer.from(protocol.render(initial, { entry: 'index.html' }));
        const packs = new Map();
        for (let i = 0; i + 1 < built.lazyPackFiles.length; i += 2) {
          packs.set(built.lazyPackFiles[i], gzipSync(built.lazyPackFiles[i + 1]));
        }
        fixtures.set(base, {
          protocol, document, script, packs, gzipDocument: gzipSync(document), gzipScript: gzipSync(script),
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
      else if (resource === '_webui' && segments[4] === 'packs' && fixture.packs.has(segments[5])) {
        bytes = fixture.packs.get(segments[5]);
        type = 'application/json';
      } else if (resource === 'state') {
        await delay(latencyMs);
        bytes = gzipSync(JSON.stringify({ value: 'Fresh' }));
        type = 'application/json';
      } else { response.writeHead(404).end(); return; }
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
