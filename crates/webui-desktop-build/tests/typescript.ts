// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { Item, Status } from '../../webui-desktop/tests/fixtures/typed-ipc/ts/application.js';
import { connectDesktop, schema, type RendererHandlers } from '../../webui-desktop/tests/fixtures/typed-ipc/ts/ipc.js';
import { defaultLimits, type IpcTransport } from '@microsoft/webui-desktop';

const item: Item = {
  id: 0xffffffffffffffffn, image: new Uint8Array([0, 255, 128, 1]), title: '',
  minimum: -0x8000000000000000n, status: 123456 as Status,
  scores: [-2147483648, 2147483647],
  labels: new Map([
    ['__proto__', 'prototype data'], ['constructor', 'constructor data'],
    ['prototype', 'ordinary data'], ['label', 'value'],
  ]),
  choice: { $case: 'number', value: 0x7fffffffffffffffn },
  chunks: new Map([[-2147483648, new Uint8Array([0, 255])]]),
  enabled: false, metrics: undefined,
  flags: new Map([[false, 'false'], [true, 'true']]),
  unsignedKeys: new Map([[0n, 'zero'], [0xffffffffffffffffn, 'maximum']]),
  signedKeys: new Map([[-0x8000000000000000n, 'minimum'], [0x7fffffffffffffffn, 'maximum']]),
};
const save = schema.methods.find(method => method.id === 1101)!;
const bytes = save.request.encode(item);
const tsGolden = new URL('../../webui-desktop/tests/fixtures/typed-ipc/golden-ts.bin', import.meta.url);
if (process.env.WEBUI_UPDATE_IPC_FIXTURE) writeFileSync(tsGolden, bytes);
assert.deepEqual(new Uint8Array(readFileSync(tsGolden)), bytes);
save.request.validate(item, defaultLimits);
save.request.validateBytes(bytes, defaultLimits);
assert.deepEqual(save.request.decode(bytes), item);
// Protobuf field order is not canonical. Compare decoding in both directions,
// and canonical re-encoding with the originating mature codec.
const golden = new Uint8Array(readFileSync(new URL('../../webui-desktop/tests/fixtures/typed-ipc/golden.bin', import.meta.url)));
assert.deepEqual(Item.decode(golden), item);
assert.deepEqual(Item.encode(Item.decode(bytes)).finish(), bytes);
assert.equal(save.response!.decode(new Uint8Array()), undefined);
assert.deepEqual(save.response!.encode(undefined), new Uint8Array());
assert.throws(() => save.request.validate({ ...item, id: 1 }, defaultLimits));
assert.throws(() => save.request.validate({ ...item, id: 0x10000000000000000n }, defaultLimits));
assert.throws(() => save.request.validate({ ...item, image: [1, 2] }, defaultLimits));
assert.throws(() => save.request.validate({ ...item, chunks: new Map([['01', new Uint8Array()]]) }, defaultLimits));
assert.throws(() => save.request.validate({ ...item, labels: { label: 'not a Map' } }, defaultLimits));
assert.throws(() => save.request.validate({ ...item, flags: new Map([['false', 'not boolean']]) }, defaultLimits));
assert.throws(() => save.request.validate({ ...item, unsignedKeys: new Map([[1, 'not bigint']]) }, defaultLimits));
assert.throws(() => save.request.validate({ ...item, unsignedKeys: new Map([[-1n, 'negative']]) }, defaultLimits));
assert.throws(() => save.request.validate({ ...item, signedKeys: new Map([[-0x8000000000000001n, 'too small']]) }, defaultLimits));
assert.throws(() => save.request.validate({ ...item, unsignedKeys: new Map([[0x10000000000000000n, 'too large']]) }, defaultLimits));
assert.throws(() => save.request.validate({ ...item, choice: { $case: 'missing', value: 1 } }, defaultLimits));
const large = { ...item, image: new Uint8Array(256 * 1024).fill(0xab), title: undefined, enabled: undefined };
save.request.validate(large, defaultLimits);
assert.deepEqual(save.request.decode(save.request.encode(large)), large);
const metrics = {
  infinity: Infinity, notANumber: NaN, maximumFixed: 0xffffffffffffffffn,
  minimumFixed: -0x8000000000000000n, minimumZigzag: -0x8000000000000000n,
  maximumFixed32: 4294967295, minimumFixed32: -2147483648,
  maximumUnsigned: 4294967295, minimumSigned: -2147483648,
  truth: true, details: [{ URLValue: 'nested' }],
};
const allScalars = { ...item, metrics };
save.request.validate(allScalars, defaultLimits);
const allBytes = save.request.encode(allScalars);
save.request.validateBytes(allBytes, defaultLimits);
assert.deepEqual(save.request.decode(allBytes), allScalars);

// Never invoked: compile-time guards on the actual generated application API.
async function typeChecks(transport: IpcTransport) {
  const renderer: RendererHandlers = { labelFor: request => ({ text: String(request.id) }) };
  const app = await connectDesktop(transport, { renderer });
  const completion: void = await app.host.save(item);
  void completion;
  await app.host.selected(item);
  const subscription = app.renderer.onChanged(value => { const id: bigint = value.id; void id; });
  subscription.close();
  // @ts-expect-error 64-bit integers must not be JS numbers.
  await app.host.save({ ...item, id: 12 });
  // @ts-expect-error 64-bit map keys must remain bigint.
  await app.host.save({ ...item, unsignedKeys: new Map([[1, 'number key']]) });
  // @ts-expect-error bool map keys must remain boolean.
  await app.host.save({ ...item, flags: new Map([['true', 'string key']]) });
  // @ts-expect-error all protobuf maps use Map, never object dictionaries.
  await app.host.save({ ...item, labels: { label: 'object map' } });
  // @ts-expect-error notifications never return application data.
  const reply: Item = await app.host.selected(item);
  // @ts-expect-error renderer calls are not host methods.
  await app.host.labelFor(item);
  // @ts-expect-error an acknowledged Empty RPC requires no Empty object.
  const empty: {} = await app.host.save(item);
  void reply; void empty;
}
void typeChecks;
console.log('generated TypeScript codecs, golden bytes, validation, and Empty passed');
