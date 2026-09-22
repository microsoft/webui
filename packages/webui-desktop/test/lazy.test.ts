// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test, { type TestContext } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { resolve, basename } from 'node:path';
import { pathToFileURL } from 'node:url';
import { build } from 'esbuild';
import type { ConnectionSchema, DesktopConnection, IpcError, IpcTransport, RequestContext, Subscription } from '../src/index.js';
import { FakeTransport, deferred, frame, Kind, turn } from './helpers.js';

const fixture = resolve('../../crates/webui-desktop/tests/fixtures/typed-ipc');
const alias = {
  '@microsoft/webui-desktop': resolve('src/index.ts'),
  '@bufbuild/protobuf/wire': resolve('node_modules/@bufbuild/protobuf/dist/esm/wire/index.js'),
};
type Item = Record<string, unknown>;
type Handlers = { labelFor(value: Item, context: RequestContext): { text: string } };
interface App extends DesktopConnection {
  host: { save(value: Item): Promise<void>; selected(value: Item): Promise<void> };
  renderer: { onChanged(callback: (value: Item) => void | Promise<void>): Subscription };
  setRenderer(handlers: Handlers): Subscription;
}
interface Facade {
  schemaHash: string;
  connectDesktop(transport: IpcTransport, options?: {
    renderer?: Handlers; onError?: (error: IpcError) => void;
  }): Promise<App>;
}

async function bundle(t: TestContext) {
  const directory = await mkdtemp(resolve('.lazy-test-'));
  const key = Symbol.for(basename(directory));
  const globals = globalThis as unknown as Record<symbol, string[]>;
  const evaluated: string[] = globals[key] = [];
  t.after(async () => { delete globals[key]; await rm(directory, { recursive: true, force: true }); });
  const result = await build({
    entryPoints: [resolve(fixture, 'ts/ipc.ts')], outdir: directory,
    bundle: true, splitting: true, format: 'esm', platform: 'browser',
    target: 'es2022', minify: true, metafile: true, alias,
    banner: { js: `globalThis[Symbol.for(${JSON.stringify(basename(directory))})].push(import.meta.url);` },
  });
  const entry = Object.entries(result.metafile.outputs).find(([, info]) => info.entryPoint?.endsWith('/ipc.ts'))!;
  assert(entry);
  assert.equal(entry[1].imports.length, 1);
  assert.equal(entry[1].imports[0]!.kind, 'dynamic-import');
  const facadeInput = Object.keys(result.metafile.inputs).find(path => path.endsWith('/ts/ipc.ts'));
  assert.deepEqual(Object.keys(entry[1].inputs), [facadeInput]);
  assert(entry[1].bytes < 512, 'the initial facade must not contain the runtime or schema codecs');
  const runtime = resolve(entry[1].imports[0]!.path);
  const entryPath = resolve(entry[0]);
  const facade = await import(pathToFileURL(entryPath).href) as Facade;
  assert.equal(evaluated.length, 1, 'importing bindings must not evaluate the private chunk');
  return { facade, runtime, evaluated };
}

test('generated facade loads once; concurrent first connections remain independent', async t => {
  const { facade, runtime, evaluated } = await bundle(t);
  assert.deepEqual(Object.keys(facade).sort(), ['connectDesktop', 'schemaHash']);
  const a = new FakeTransport();
  const b = new FakeTransport();
  const gate = deferred<void>();
  let starts = 0;
  const start = a.start.bind(a);
  a.start = async (hello, receiver) => {
    starts++;
    assert.deepEqual(Reflect.ownKeys(hello as object).sort(), ['contractMajor', 'contractName', 'schemaHash', 'wireVersion']);
    assert.equal((hello as { schemaHash: string }).schemaHash, facade.schemaHash);
    await gate.promise;
    return start(hello, receiver);
  };
  const handlers: Handlers = { labelFor: value => ({ text: String(value.id) }) };
  const pending = facade.connectDesktop(a, { renderer: handlers });
  const second = await facade.connectDesktop(b, { renderer: handlers });
  t.after(() => second.close());
  assert.equal(starts, 1);
  assert.equal(evaluated.length, 2);
  assert.notEqual(a.receiver, b.receiver);
  gate.resolve();
  const first = await pending;
  t.after(() => first.close());
  assert.notEqual(first, second);
  assert.notEqual(first.closed, second.closed);
  const { schema } = await import(pathToFileURL(runtime).href) as { schema: ConnectionSchema };
  const codec = schema.methods.find(method => method.id === 1101)!.request;
  const value = codec.decode(new Uint8Array(await readFile(resolve(fixture, 'golden.bin')))) as Item;
  let completed = false;
  const saving = first.host.save(value).then(() => { completed = true; });
  const notifying = second.host.selected(value);
  await turn();
  assert.equal(a.sent[0]!.id, 1n);
  assert.equal(b.sent[0]!.id, 1n, 'each connection has its own call IDs');
  assert.deepEqual(codec.decode((a.sent[0]!.body as { value: Uint8Array }).value), value);
  await b.receive(frame(1n, 1n, Kind.ACCEPT));
  await notifying;
  assert.equal(completed, false);
  await a.receive({ ...frame(1n, 1n, Kind.RESULT), body: { $case: 'payload', value: new Uint8Array() } });
  await saving;
  first.close();
  assert.equal((await first.closed).code, 'closed');
  assert.equal(b.closed, false);
  await assert.rejects(first.host.save(value), { code: 'closed' });
  second.close();
  const third = await facade.connectDesktop(new FakeTransport());
  third.close();
  assert.equal(evaluated.length, 2, 'explicit reconnect must reuse only module evaluation, not connections');
});

test('lazy generated handlers, notifications, subscriptions and onError retain their semantics', async t => {
  const { facade, runtime } = await bundle(t);
  const transport = new FakeTransport();
  const errors: IpcError[] = [];
  const connection = await facade.connectDesktop(transport, { onError: error => errors.push(error) });
  t.after(() => connection.close());
  const { schema } = await import(pathToFileURL(runtime).href) as { schema: ConnectionSchema };
  const codec = schema.methods.find(method => method.id === 1101)!.request;
  const bytes = new Uint8Array(await readFile(resolve(fixture, 'golden.bin')));
  const value = codec.decode(bytes) as Item;
  const handlers = connection.setRenderer({ labelFor(request, context) {
    assert.deepEqual(request, value);
    assert.equal(context.signal.aborted, false);
    return { text: String(request.id) };
  } });
  assert.throws(() => connection.setRenderer({ labelFor: () => ({ text: 'duplicate' }) }), { code: 'invalid-payload' });
  await transport.receive({ ...frame(1n, 1n, Kind.REQUEST), methodId: 2001, timeoutMs: 30000, body: { $case: 'payload', value: bytes } });
  await turn();
  const result = transport.sent.find(message => message.kind === Kind.RESULT)!;
  const response = schema.methods.find(method => method.id === 2001)!.response!;
  assert.deepEqual(response.decode((result.body as { value: Uint8Array }).value), { text: String(value.id) });
  handlers.close();
  const replacement = connection.setRenderer({ labelFor: () => ({ text: 'replacement' }) });
  replacement.close();
  let callbacks = 0;
  const subscription = connection.renderer.onChanged(payload => {
    callbacks++;
    assert.deepEqual(payload, value);
    throw new Error('private callback error');
  });
  const notify = (id: bigint) => transport.receive({
    ...frame(1n, id, Kind.NOTIFY), methodId: 2002, body: { $case: 'payload' as const, value: bytes },
  });
  await notify(2n);
  await turn();
  assert.equal(callbacks, 1);
  assert.equal(errors[0]!.code, 'handler');
  assert(transport.sent.some(message => message.id === 2n && message.kind === Kind.ACCEPT));
  subscription.close();
  await notify(3n);
  await turn();
  assert.equal(callbacks, 1);
});

test('lazy load does not cache failed handshakes or bypass handler validation', async t => {
  const { facade, evaluated } = await bundle(t);
  const invalid = new FakeTransport();
  invalid.start = async () => { assert.fail('invalid handlers reached the transport'); };
  await assert.rejects(facade.connectDesktop(invalid, { renderer: {} as Handlers }), { code: 'invalid-payload' });
  const failed = new FakeTransport();
  let starts = 0;
  failed.start = async () => { starts++; throw new Error('hello failed'); };
  const rejection = assert.rejects(facade.connectDesktop(failed), { code: 'transport' });
  const healthy = await facade.connectDesktop(new FakeTransport());
  await rejection;
  assert.equal(starts, 1, 'no automatic handshake retry');
  assert.equal(failed.closed, true);
  healthy.close();
  assert.equal(evaluated.length, 2);
});

for (const failure of ['missing', 'evaluation'] as const) {
  test(`lazy ${failure} failure rejects every explicit connect without retry or transport activation`, async t => {
    const { facade, runtime, evaluated } = await bundle(t);
    const original = await readFile(runtime);
    if (failure === 'missing') await rm(runtime);
    else await writeFile(runtime, `throw new Error('module evaluation failed');\n${original.toString()}`);
    const transport = new FakeTransport();
    transport.start = async () => { assert.fail('failed module load reached the transport'); };
    const results = await Promise.allSettled([facade.connectDesktop(transport), facade.connectDesktop(transport)]);
    assert.equal(results[0]!.status, 'rejected');
    assert.deepEqual(results[0], results[1]);
    await writeFile(runtime, original);
    const [later] = await Promise.allSettled([facade.connectDesktop(transport)]);
    assert.deepEqual(later, results[0], 'a rejected loader stays rejected rather than silently retrying');
    assert.equal(evaluated.length, 1);
  });
}

test('type-only generated imports have no runtime dependency', async () => {
  const result = await build({
    stdin: {
      contents: `import type { AppConnection, HostClient, RendererHandlers, RendererEvents } from ${JSON.stringify(resolve(fixture, 'ts/ipc.ts'))};
export type Application = [AppConnection, HostClient, RendererHandlers, RendererEvents];`,
      resolveDir: resolve('.'), loader: 'ts',
    },
    bundle: true, write: false, format: 'esm', platform: 'browser', metafile: true,
  });
  assert.equal(result.outputFiles[0]!.text.trim(), '');
  assert.deepEqual(Object.keys(result.metafile.inputs), ['<stdin>']);
});
