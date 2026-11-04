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
  'test-owner.html': '<h1>Owner</h1><if condition="show" lazy="idle"><p>{{message}}</p><aside>Inline</aside><lazy-card></lazy-card></if>',
  'lazy-card.html': '<p>Card</p>',
  'lazy-card.ts': 'export {};',
};
const closedState = '{"show":false,"message":"Hidden"}';
const openState = '{"show":true,"message":"Fresh"}';

/** The parts of a pack that WASM and native builds both report. */
function portable(pack) {
  const { brotliBytes, ...rest } = pack;
  return rest;
}

const directory = mkdtempSync(join(tmpdir(), 'webui-wasm-lazy-'));
try {
  for (const [name, content] of Object.entries(files)) {
    writeFileSync(join(directory, name), content);
  }
  // WASM builds inline component CSS, so the native build uses the same strategy.
  const native = build({ appDir: directory, plugin: 'webui', css: 'style' });
  assert.equal(native.lazyPackFiles.length, 1);
  const packUrl = native.lazyPackFiles[0].path;
  assert.equal(packUrl, `_webui/packs/${native.lazyPackFiles[0].fileName}`);
  assert.deepEqual(native.lazyPackFiles[0].scripts, ['./lazy-card.ts']);
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
  assert.deepEqual(app.lazyPackFiles.map(portable), native.lazyPackFiles.map(portable));
  assert.equal(app.lazyPackFiles[0].brotliBytes, undefined);
  assert.deepEqual(app.lazyPackFiles[0].stylesheets, [], 'WASM builds inline component CSS');
  assert.deepEqual(app.lazyScriptsModule, native.lazyScriptsModule);
  assert.equal(app.lazyScriptsModule.declaration.fileName, 'lazy.webui.d.ts');
  assert.deepEqual(app.warnings, []);
  const browser = new combined.Protocol(app.protocol, 'webui');
  try {
    const closed = browser.render(closedState);
    assert.ok(closed.includes(packUrl));
    assert.ok(!closed.includes('Inline'));
    assert.ok(browser.render(openState).includes('Inline'));
  } finally {
    browser.free();
  }
  const cdn = combined.buildApp(files, 'index.html', { assetBase: 'https://cdn.example.com/app/' });
  assert.deepEqual(cdn.lazyPackFiles.map(portable), native.lazyPackFiles.map(portable));
  const cdnProtocol = new combined.Protocol(cdn.protocol, 'webui');
  try {
    // Pack paths stay relative; the browser resolves them against assetBase.
    const closed = cdnProtocol.render(closedState);
    assert.ok(closed.includes(`"${packUrl}"`));
    assert.ok(closed.includes('"assetBase":"https://cdn.example.com/app/"'));
    assert.ok(
      closed.includes('<meta name="webui-asset-base" content="https:&#x2F;&#x2F;cdn.example.com&#x2F;app&#x2F;">'),
    );
  } finally {
    cdnProtocol.free();
  }
  const mapped = combined.buildApp(files, 'index.html', {
    modulesOut: 'generated',
    componentModules: { 'lazy-card': '@contoso/cards/lazy-card.js' },
  });
  assert.deepEqual(mapped.lazyPackFiles[0].scripts, ['@contoso/cards/lazy-card.js']);
  assert.match(mapped.lazyScriptsModule.content, /"lazy-card": \(\) => import\("@contoso\/cards\/lazy-card\.js"\),/);
  const relocated = combined.buildApp(files, 'index.html', { modulesOut: 'generated' });
  assert.deepEqual(relocated.lazyPackFiles[0].scripts, ['../lazy-card.ts']);
  assert.throws(
    () => combined.buildApp(files, 'index.html', { assetBase: 'packs' }),
    error => String(error).includes('invalid asset base "packs"'),
  );
  assert.throws(
    () => combined.buildApp(files, 'index.html', { componentModules: { 'lazy-card': 'a b.js' } }),
    error => String(error).includes('invalid-component-module'),
  );
  assert.throws(
    () => combined.buildApp(files, 'index.html', { assetbase: '/static/' }),
    error => String(error).includes('invalid buildApp options'),
  );
  console.log('WASM buildApp emits the same packs and lazy.webui.js as the native build.');
} finally {
  rmSync(directory, { recursive: true, force: true });
}
