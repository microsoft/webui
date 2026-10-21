// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { build, Protocol } from '../dist/index.js';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const wasmDirectory = join(repository, 'docs', '.webui-press', 'public', 'wasm', 'handler');
const wasm = await import(pathToFileURL(join(wasmDirectory, 'webui_wasm_handler.js')).href);
wasm.initSync({ module: readFileSync(join(wasmDirectory, 'webui_wasm_handler_bg.wasm')) });

const directory = mkdtempSync(join(tmpdir(), 'webui-wasm-deferred-'));
try {
  writeFileSync(join(directory, 'index.html'), '<html><head></head><body><test-owner></test-owner></body></html>');
  writeFileSync(join(directory, 'test-owner.html'),
    '<if condition="show" defer id="details"><p>{{message}}</p><aside>Inline</aside></if>');
  const result = build({ appDir: directory, plugin: 'webui' });
  const native = new Protocol(result.protocol, { plugin: 'webui' });
  const compiled = new wasm.Protocol(result.protocol, 'webui');
  try {
    const expected = native.prepareDeferredResources('test-owner', 'details');
    const actual = compiled.prepareDeferredResources('test-owner', 'details');
    assert.deepEqual(actual, expected);
    assert.deepEqual(
      JSON.parse(compiled.renderDeferredResources('test-owner', 'details')),
      JSON.parse(expected.json),
    );
    const state = '{"message":"Fresh","unused":42}';
    assert.deepEqual(
      JSON.parse(compiled.renderDeferredPartial(state, 'test-owner', 'details', actual.options)),
      JSON.parse(native.renderDeferredPartial(state, 'test-owner', 'details', expected.options)),
    );
    assert.throws(
      () => compiled.renderDeferredResources('test-owner', 'missing'),
      error => String(error).includes('deferred-not-found'),
    );
    console.log('WASM and Node deferred resource/state responses agree.');
  } finally {
    compiled.free();
  }
  const combinedDirectory = join(repository, 'docs', '.webui-press', 'public', 'wasm', 'all');
  const combined = await import(pathToFileURL(join(combinedDirectory, 'webui_wasm_all.js')).href);
  combined.initSync({ module: readFileSync(join(combinedDirectory, 'webui_wasm_all_bg.wasm')) });
  const virtual = {
    'index.html': readFileSync(join(directory, 'index.html'), 'utf8'),
    'test-owner.html': readFileSync(join(directory, 'test-owner.html'), 'utf8'),
  };
  const browserProtocol = combined.build_protocol(virtual, 'index.html');
  const browser = new combined.Protocol(browserProtocol, 'webui');
  const browserOnNative = new Protocol(browserProtocol, { plugin: 'webui' });
  try {
    const prepared = browser.prepareDeferredResources('test-owner', 'details');
    assert.deepEqual(prepared, browserOnNative.prepareDeferredResources('test-owner', 'details'));
    assert.ok(prepared.options.buildId.startsWith('sha256:'));
    assert.ok(!browser.render('{"show":false}').includes('Inline'));
    assert.deepEqual(
      JSON.parse(browser.renderDeferredPartial('{"message":"Browser"}', 'test-owner', 'details', prepared.options)),
      JSON.parse(browserOnNative.renderDeferredPartial('{"message":"Browser"}', 'test-owner', 'details', prepared.options)),
    );
    console.log('WASM-built deferred protocols retain the same host contract.');
  } finally {
    browser.free();
  }
} finally {
  rmSync(directory, { recursive: true, force: true });
}
