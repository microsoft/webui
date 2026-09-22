// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { defaultLimits, type IpcLimits, type BinaryReceiver, type IpcTransport, type MessageCodec, type RpcDescriptor, type EventDescriptor, type IpcSchema, validateMessage, validateValue, type MessageShape } from '../src/index.js';
import { IpcFrame, Kind, frame } from '../src/framing.js';
import { Item, Empty } from './fixture.js';
export { Item, IpcFrame, Kind, frame };

export function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((a, b) => { resolve = a; reject = b; });
  return { promise, resolve, reject };
}
export const shapes: MessageShape[] = [{ fields: [
  { number: 1, name: 'id', kind: 'uint64', repeated: false, optional: false, mapKey: false },
  { number: 2, name: 'signed', kind: 'sint64', repeated: false, optional: false, mapKey: false },
  { number: 3, name: 'data', kind: 'bytes', repeated: false, optional: false, mapKey: false },
] }];
export const itemCodec: MessageCodec<Item> = {
  encode: value => Item.encode(value).finish(),
  decode: bytes => Item.decode(bytes),
  validate: (value, limits) => validateValue(value, 0, shapes, limits),
  validateBytes: (bytes, limits) => validateMessage(bytes, 0, shapes, limits),
};
export const voidCodec: MessageCodec<void> = {
  encode: () => Empty.encode({}).finish(),
  decode: bytes => { Empty.decode(bytes); },
  validate: value => { if (value !== undefined) throw new Error('Expected void'); },
  validateBytes: (bytes, limits) => validateMessage(bytes, 0, [{ fields: [] }], limits),
};
export const save: RpcDescriptor<Item, void, 'host'> = { id: 1101, kind: 'rpc', receiver: 'host', request: itemCodec, response: voidCodec };
export const selected: EventDescriptor<Item, 'host'> = { id: 1102, kind: 'notification', receiver: 'host', request: itemCodec };
export const label: RpcDescriptor<Item, Item, 'renderer'> = { id: 2001, kind: 'rpc', receiver: 'renderer', request: itemCodec, response: itemCodec };
export const changed: EventDescriptor<Item, 'renderer'> = { id: 2002, kind: 'notification', receiver: 'renderer', request: itemCodec };
export const schema: IpcSchema = { wireVersion: 2, contractName: 'test', contractMajor: 1, schemaHash: 'a'.repeat(64), methods: [save, selected, label, changed] };
export const item: Item = { id: 0xffffffffffffffffn, signed: -0x8000000000000000n, data: new Uint8Array([0, 128, 255]) };

export class FakeTransport implements IpcTransport {
  receiver!: BinaryReceiver;
  sent: IpcFrame[] = [];
  closed = false;
  limits: IpcLimits = { ...defaultLimits };
  accept?: () => Promise<void>;
  onSend?: (frame: IpcFrame) => void;
  async start(_hello: unknown, receiver: BinaryReceiver) {
    this.receiver = receiver;
    return { generation: '1', token: 'a'.repeat(32), limits: this.limits };
  }
  async send(bytes: Uint8Array) {
    const value = IpcFrame.decode(bytes);
    this.sent.push(value);
    this.onSend?.(value);
    await this.accept?.();
  }
  close() { this.closed = true; }
  receive(value: IpcFrame) { return this.receiver.receive(IpcFrame.encode(value).finish()); }
}
export function invocation(id: bigint, kind: Kind, methodId: number): IpcFrame {
  return { ...frame(1n, id, kind), methodId, timeoutMs: kind === Kind.REQUEST ? 30000 : 0, body: { $case: 'payload', value: itemCodec.encode(item) } };
}
/** Wait for a real event-loop turn, not a wall-clock sleep or retry loop. */
export const turn = () => new Promise<void>(resolve => setImmediate(resolve));
