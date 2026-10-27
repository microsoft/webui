// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { build, Protocol } from '../dist/index.js';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const wasmRoot = join(repository, 'docs', '.webui-press', 'public', 'wasm');

async function loadWasm(variant) {
  const directory = join(wasmRoot, variant);
  const module = await import(pathToFileURL(join(directory, `webui_wasm_${variant}.js`)).href);
  module.initSync({ module: readFileSync(join(directory, `webui_wasm_${variant}_bg.wasm`)) });
  return module;
}

const files = {
  'index.html': '<html><head></head><body><test-owner></test-owner></body></html>',
  'test-owner.html': '<h1>Owner</h1><if condition="show" lazy="idle"><p>{{message}}</p><aside>Inline</aside></if>',
};
const closedState = '{"show":false,"message":"Hidden"}';
const openState = '{"show":true,"message":"Fresh"}';

const directory = mkdtempSync(join(tmpdir(), 'webui-wasm-lazy-'));
try {
  for (const [name, content] of Object.entries(files)) {
    writeFileSync(join(directory, name), content);
  }
  const native = build({ appDir: directory, plugin: 'webui' });
  assert.equal(native.lazyPackFiles.length, 2);
  const packUrl = `_webui/packs/${native.lazyPackFiles[0]}`;
  const nativeProtocol = new Protocol(native.protocol, { plugin: 'webui' });
  const nativeClosed = nativeProtocol.render(JSON.parse(closedState)).toString('utf8');
  assert.ok(nativeClosed.includes(packUrl));
  assert.ok(!nativeClosed.includes('Inline'));

  const handler = await loadWasm('handler');
  const rendered = new handler.Protocol(native.protocol, 'webui');
  try {
    assert.equal(rendered.render(closedState), nativeClosed);
    assert.equal(
      rendered.render(openState),
      nativeProtocol.render(JSON.parse(openState)).toString('utf8'),
    );
    console.log('WASM and Node render the same lazy-region SSR and pack references.');
  } finally {
    rendered.free();
  }

  const combined = await loadWasm('all');
  assert.throws(
    () => combined.build_protocol(files, 'index.html'),
    error => String(error).includes('buildApp'),
  );
  const app = combined.buildApp(files, 'index.html');
  assert.deepEqual(app.lazyPackFiles, native.lazyPackFiles);
  const browser = new combined.Protocol(app.protocol, 'webui');
  try {
    const closed = browser.render(closedState);
    assert.ok(closed.includes(packUrl));
    assert.ok(!closed.includes('Inline'));
    assert.ok(browser.render(openState).includes('Inline'));
  } finally {
    browser.free();
  }
  const cdn = combined.buildApp(files, 'index.html', undefined, undefined, 'https://cdn.example.com/packs/');
  assert.deepEqual(cdn.lazyPackFiles, native.lazyPackFiles);
  const cdnProtocol = new combined.Protocol(cdn.protocol, 'webui');
  try {
    assert.ok(cdnProtocol.render(closedState).includes(`https://cdn.example.com/packs/${native.lazyPackFiles[0]}`));
  } finally {
    cdnProtocol.free();
  }
  assert.throws(
    () => combined.buildApp(files, 'index.html', undefined, undefined, 'packs'),
    error => String(error).includes("invalid pack public base 'packs'"),
  );
  console.log('WASM buildApp emits the same packs as the native build.');
} finally {
  rmSync(directory, { recursive: true, force: true });
}
