// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { runInNewContext } from 'node:vm';
import { build } from 'esbuild';
import { defaultLimits, type NativeIpcBootstrap } from '../src/index.js';
import { schema, save, item, frame, IpcFrame, Kind, turn } from './helpers.js';

test('built runtime and bootstrap admit exactly nine native hello fields and complete Save', async () => {
  for (const name of ['native-bootstrap.js', 'desktop-runtime.js']) {
    const bundled = readFileSync(`dist/${name}`);
    const previous = readFileSync(resolve('../../crates/webui-desktop/src/generated/ipc', name));
    assert(bundled.byteLength <= previous.byteLength + 1024, `Default artifact grew unexpectedly: ${name}`);
    assert.equal(bundled.includes(Buffer.from('ipcDataResult')), false);
  }
  const artifact = readFileSync('dist/native-bootstrap.js', 'utf8');
  const runtime = await import(pathToFileURL(resolve('dist/desktop-runtime.js')).href) as typeof import('../src/index.js');
  const helloKeys = ['kind', 'callId', 'wireVersion', 'contractName', 'contractMajor', 'schemaHash',
    'navigation', 'documentNonce', 'challenge'].sort();
  for (const platform of ['webkit', 'webview2']) {
    const messages: Record<string, unknown>[] = [];
    let nativeListener: ((event: { data: unknown }) => void) | undefined;
    const target: Record<string, any> = {};
    target.top = target;
    const events = new EventTarget();
    target.addEventListener = events.addEventListener.bind(events);
    const postMessage = (message: Record<string, unknown>) => {
      messages.push(message);
      if (message.kind !== 'hello') return undefined;
      // Match native's strict dictionary admission, rather than tolerating
      // local schema descriptors or properties dropped by JSON.stringify.
      assert.deepEqual(Reflect.ownKeys(message).sort(), helloKeys);
      const serialized = JSON.parse(JSON.stringify(message)) as Record<string, unknown>;
      assert.deepEqual(Object.keys(serialized).sort(), helloKeys);
      for (const key of helloKeys) {
        assert.equal(typeof serialized[key], key === 'wireVersion' || key === 'contractMajor' ? 'number' : 'string');
      }
      const reply = {
        kind: 'helloResult', callId: message.callId, navigation: message.navigation,
        documentNonce: message.documentNonce, challenge: message.challenge,
        generation: '1', token: 'a'.repeat(32), limits: defaultLimits,
      };
      if (platform === 'webkit') return Promise.resolve(reply);
      queueMicrotask(() => nativeListener?.({ data: reply }));
      return undefined;
    };
    if (platform === 'webkit') target.webkit = { messageHandlers: { webuiDesktopIpc: { postMessage } } };
    else target.chrome = { webview: {
      postMessage,
      addEventListener(_name: string, listener: typeof nativeListener) { nativeListener = listener; },
      removeEventListener() { nativeListener = undefined; },
    } };
    runInNewContext(artifact, { window: target, crypto, TextEncoder, setTimeout, clearTimeout, Uint8Array });
    const bootstrap = target.__webuiDesktopIpcV2 as NativeIpcBootstrap;
    assert.equal(Object.isFrozen(bootstrap), true);
    assert.equal(typeof target.__webuiDesktopIpcReceiveV2, 'function');
    const outbound: Uint8Array<ArrayBuffer>[] = [];
    let saves = 0;
    const transport = runtime.createDesktopTransport({ bootstrap, fetch: (async (_path, init) => {
      if (init?.method === 'POST') {
        assert(init.body instanceof Uint8Array);
        const request = IpcFrame.decode(init.body);
        assert.equal(request.kind, Kind.REQUEST);
        assert.equal(request.methodId, save.id);
        saves++;
        outbound.push(IpcFrame.encode({ ...frame(1n, request.id, Kind.RESULT),
          body: { $case: 'payload', value: new Uint8Array() },
        }).finish());
        const ready = { kind: 'ready', generation: '1' };
        if (platform === 'webkit') target.__webuiDesktopIpcReceiveV2(ready);
        else nativeListener?.({ data: ready });
        return new Response(null, { status: 204 });
      }
      const bytes = outbound.shift();
      return bytes ? new Response(bytes, { headers: { 'Content-Type': 'application/x-protobuf' } })
        : new Response(null, { status: 204 });
    }) as typeof fetch });
    const hello = runtime.createConnection(schema, transport);
    await turn();
    assert.equal(messages.length, 0);
    assert.equal(bootstrap.activate({ navigation: '1', documentNonce: 'b'.repeat(32), challenge: 'c'.repeat(32) }), false);
    assert.equal(bootstrap.activate({ navigation: '1', documentNonce: bootstrap.documentNonce, challenge: 'c'.repeat(32) }), true);
    const connection = await hello;
    assert.equal(messages.length, 1);
    assert.equal(await connection.call(save, item), undefined);
    assert.equal(saves, 1);
    connection.close();
    assert.equal(messages[1]!.kind, 'disconnect');
    assert.equal(messages[1]!.token, 'a'.repeat(32));
    assert.equal(nativeListener, undefined);
  }
});

test('only the dedicated self-contained local artifacts negotiate and deliver native frames', async () => {
  for (const name of ['local-native-bootstrap.js', 'local-desktop-runtime.js']) {
    const built = readFileSync(`dist/${name}`);
    const checkedIn = readFileSync(resolve('../../crates/webui-desktop/src/generated/ipc', name));
    assert.deepEqual(built, checkedIn);
  }
  const bundledRuntime = readFileSync('dist/desktop-runtime.js', 'utf8');
  const bundledBootstrap = readFileSync('dist/native-bootstrap.js', 'utf8');
  assert.equal(bundledRuntime.includes('ipcDataResult'), false);
  assert.equal(bundledBootstrap.includes('ipcDataResult'), false);
  const bootstrapScript = readFileSync('dist/local-native-bootstrap.js', 'utf8');
  const runtime = await import(pathToFileURL(resolve('dist/local-desktop-runtime.js')).href) as typeof import('../src/index.js');
  for (const platform of ['webkit', 'webview2']) {
    const target: Record<string, unknown> = {};
    target.top = target;
    const events = new EventTarget();
    target.addEventListener = events.addEventListener.bind(events);
    let hostListener: ((event: { data: unknown }) => void) | undefined;
    const sent: Record<string, unknown>[] = [];
    const control = (message: Record<string, unknown>) => {
      if (message.kind !== 'hello') return undefined;
      assert.equal(message.nativeCarrierVersion, 1);
      const reply = { kind: 'helloResult', callId: message.callId, navigation: message.navigation,
        documentNonce: message.documentNonce, challenge: message.challenge,
        generation: '1', token: 'a'.repeat(32), limits: defaultLimits, nativeCarrierVersion: 1 };
      if (platform === 'webkit') return Promise.resolve(reply);
      queueMicrotask(() => hostListener?.({ data: reply }));
      return undefined;
    };
    const data = (message: Record<string, unknown>) => {
      sent.push(message);
      const count = atob(message.data as string).length;
      const offset = message.offset as number;
      const reply = { kind: 'ipcDataResult', version: 1, callId: message.callId,
        operation: 'send', generation: '1', nextOffset: offset + count,
        complete: offset + count === message.totalBytes };
      if (platform === 'webkit') return Promise.resolve(reply);
      queueMicrotask(() => hostListener?.({ data: reply }));
      return undefined;
    };
    if (platform === 'webkit') target.webkit = { messageHandlers: {
      webuiDesktopIpc: { postMessage: control }, webuiDesktopIpcData: { postMessage: data },
    } };
    else target.chrome = { webview: {
      postMessage(message: Record<string, unknown>) {
        return message.kind === 'ipcData' ? data(message) : control(message);
      },
      addEventListener(_name: string, listener: typeof hostListener) { hostListener = listener; },
      removeEventListener() { hostListener = undefined; },
    } };
    runInNewContext(bootstrapScript, { window: target, crypto, TextEncoder, setTimeout, clearTimeout, Uint8Array });
    const bootstrap = target.__webuiDesktopIpcV2 as NativeIpcBootstrap;
    const transport = runtime.createDesktopTransport({ bootstrap,
      fetch: (async () => assert.fail('Local carrier must not fetch HTTP')) as typeof fetch });
    const connecting = transport.start(schema, { async receive() {}, closed() {} });
    assert.equal(bootstrap.nativeData, undefined);
    assert.equal(bootstrap.activate({ navigation: '1', documentNonce: bootstrap.documentNonce,
      challenge: 'c'.repeat(32), nativeCarrierVersion: 1 }), true);
    assert.equal((await connecting).nativeCarrierVersion, 1);
    const bytes = IpcFrame.encode(frame(1n, 1n, Kind.CANCEL)).finish();
    await transport.send(bytes);
    assert.equal(sent.length, 1);
    assert.equal(sent[0]!.kind, 'ipcData');
    assert.equal(sent[0]!.token, 'a'.repeat(32));
    transport.close();
    assert.equal(hostListener, undefined);
  }
});

test('consumer root excludes native carrier; explicit native export resolves all shipped modules', async () => {
  const native = await import('@microsoft/webui-desktop/native');
  assert.equal(typeof native.createNativeDesktopTransport, 'function');
  const outputs = [];
  for (const specifier of ['@microsoft/webui-desktop', '@microsoft/webui-desktop/native']) {
    const result = await build({
      stdin: { contents: `import { ${specifier.endsWith('/native')
        ? 'createNativeDesktopTransport' : 'createDesktopTransport'} as connect } from '${specifier}'; export const transport = connect();`,
      resolveDir: process.cwd(), sourcefile: 'consumer.ts', loader: 'ts' },
      bundle: true, platform: 'browser', format: 'esm', target: 'es2022', minify: true,
      write: false, metafile: true,
    });
    const inputs = Object.keys(result.metafile.inputs);
    const carrier = inputs.some(path => path.endsWith('native-carrier.js'));
    assert.equal(carrier, specifier.endsWith('/native'));
    assert(!result.outputFiles[0]!.text.includes('import('), 'consumer output must be self-contained');
    outputs.push(result.outputFiles[0]!.contents.length);
  }
  assert(outputs[0]! < outputs[1]!);
});
