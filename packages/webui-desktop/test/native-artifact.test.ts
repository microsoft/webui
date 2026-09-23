// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { runInNewContext } from 'node:vm';
import { defaultLimits, type NativeIpcBootstrap } from '../src/index.js';
import { schema, save, item, frame, IpcFrame, Kind, turn } from './helpers.js';

test('built runtime and bootstrap admit exactly nine native hello fields and complete Save', async () => {
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
