// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { BinaryReader } from '@bufbuild/protobuf/wire';
import { IpcError } from './errors.js';

/** Raw bytes remain available to reject overflows before codec narrowing. */
export class BoundaryReader extends BinaryReader {
  constructor(readonly bytesView: Uint8Array) { super(bytesView); }
  override tag(): ReturnType<BinaryReader['tag']> {
    guardVarint(this);
    return super.tag();
  }
}

function guardVarint(reader: BoundaryReader): void {
  for (let index = 0; index < 10; index++) {
    const byte = reader.bytesView[reader.pos + index];
    if (byte === undefined || (index === 9 && byte > 1)) throw new IpcError('invalid-payload');
    if (byte < 128) return;
  }
  throw new IpcError('invalid-payload');
}

export function readUint64(reader: BoundaryReader): bigint {
  guardVarint(reader);
  return BigInt(reader.uint64());
}

/** Protobuf uint32 validation without the codec's intentional narrowing behavior. */
export function readUint32(reader: BoundaryReader): number {
  const value = readUint64(reader);
  if (value > 0xffffffffn) throw new IpcError('invalid-payload');
  return Number(value);
}

export function readDelimited(reader: BoundaryReader): Uint8Array {
  const start = reader.pos;
  const length = readUint32(reader);
  if (length > reader.len - reader.pos) throw new IpcError('invalid-payload');
  reader.pos = start;
  return reader.bytes();
}

export function skipField(reader: BoundaryReader, type: number): void {
  if (type === 0) { readUint64(reader); return; }
  if (type === 2) { readDelimited(reader); return; }
  if (type !== 1 && type !== 5) throw new IpcError('invalid-payload');
  reader.skip(type);
}
