// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { IpcError } from './errors.js';

const utf8 = new TextEncoder();
const utf8Decoder = new TextDecoder('utf-8', { fatal: true });

export class PayloadWriter {
  private bytes: number[] = [];

  finish(): Uint8Array { return new Uint8Array(this.bytes); }
  bool(number: number, value: boolean): this { this.varintField(number, value ? 1n : 0n); return this; }
  uint32(number: number, value: number): this { this.varintField(number, BigInt(value >>> 0)); return this; }
  int32(number: number, value: number): this { this.varintField(number, BigInt.asUintN(64, BigInt(value))); return this; }
  enum(number: number, value: number): this { this.int32(number, value); return this; }
  uint64(number: number, value: bigint): this { this.varintField(number, value); return this; }
  int64(number: number, value: bigint): this { this.varintField(number, BigInt.asUintN(64, value)); return this; }
  sint32(number: number, value: number): this { this.varintField(number, BigInt((value << 1) ^ (value >> 31)) & 0xffffffffn); return this; }
  sint64(number: number, value: bigint): this { this.varintField(number, BigInt.asUintN(64, (value << 1n) ^ (value >> 63n))); return this; }
  fixed32(number: number, value: number): this { this.key(number, 5); this.u32(value >>> 0); return this; }
  sfixed32(number: number, value: number): this { this.fixed32(number, value); return this; }
  fixed64(number: number, value: bigint): this { this.key(number, 1); this.u64(value); return this; }
  sfixed64(number: number, value: bigint): this { this.fixed64(number, BigInt.asUintN(64, value)); return this; }
  float(number: number, value: number): this { this.key(number, 5); const b = new Uint8Array(4); new DataView(b.buffer).setFloat32(0, value, true); this.raw(b); return this; }
  double(number: number, value: number): this { this.key(number, 1); const b = new Uint8Array(8); new DataView(b.buffer).setFloat64(0, value, true); this.raw(b); return this; }
  string(number: number, value: string): this { this.bytesField(number, utf8.encode(value)); return this; }
  bytesField(number: number, value: Uint8Array): this { this.key(number, 2); this.varint(BigInt(value.byteLength)); this.raw(value); return this; }

  private varintField(number: number, value: bigint): void { this.key(number, 0); this.varint(value); }
  private key(number: number, wire: number): void { this.varint((BigInt(number) << 3n) | BigInt(wire)); }
  private varint(value: bigint): void {
    let current = BigInt.asUintN(64, value);
    while (current >= 0x80n) {
      this.bytes.push(Number(current & 0x7fn) | 0x80);
      current >>= 7n;
    }
    this.bytes.push(Number(current));
  }
  private u32(value: number): void {
    this.bytes.push(value & 0xff, (value >>> 8) & 0xff, (value >>> 16) & 0xff, (value >>> 24) & 0xff);
  }
  private u64(value: bigint): void {
    let current = BigInt.asUintN(64, value);
    for (let i = 0; i < 8; i++) {
      this.bytes.push(Number(current & 0xffn));
      current >>= 8n;
    }
  }
  private raw(value: Uint8Array): void {
    for (let i = 0; i < value.byteLength; i++) this.bytes.push(value[i]!);
  }
}

/** Raw bytes remain available to reject overflows before codec narrowing. */
export class BoundaryReader {
  readonly len: number;
  pos = 0;

  constructor(readonly bytesView: Uint8Array) { this.len = bytesView.byteLength; }

  tag(): [number, number] {
    const key = this.uint64();
    const number = Number(key >> 3n);
    if (number <= 0 || number > 0x1fffffff) throw new IpcError('invalid-payload');
    return [number, Number(key & 7n)];
  }

  uint64(): bigint {
    guardVarint(this);
    let value = 0n;
    let shift = 0n;
    while (true) {
      const byte = this.readByte();
      value |= BigInt(byte & 0x7f) << shift;
      if (byte < 0x80) return value;
      shift += 7n;
    }
  }

  fixed32(): number { return this.readFixed32(); }
  sfixed32(): number { return this.readFixed32() | 0; }
  fixed64(): bigint { return this.readFixed64(); }
  sfixed64(): bigint { return BigInt.asIntN(64, this.readFixed64()); }
  float(): number { const view = this.view(4); const value = view.getFloat32(0, true); this.pos += 4; return value; }
  double(): number { const view = this.view(8); const value = view.getFloat64(0, true); this.pos += 8; return value; }

  bytes(): Uint8Array {
    const length = readUint32(this);
    if (length > this.len - this.pos) throw new IpcError('invalid-payload');
    const start = this.pos;
    this.pos += length;
    return this.bytesView.subarray(start, this.pos);
  }

  skip(type: number): void {
    if (type === 0) { this.uint64(); return; }
    if (type === 1) { this.pos += 8; if (this.pos > this.len) throw new IpcError('invalid-payload'); return; }
    if (type === 2) { this.bytes(); return; }
    if (type === 5) { this.pos += 4; if (this.pos > this.len) throw new IpcError('invalid-payload'); return; }
    throw new IpcError('invalid-payload');
  }

  private readByte(): number {
    const byte = this.bytesView[this.pos];
    if (byte === undefined) throw new IpcError('invalid-payload');
    this.pos++;
    return byte;
  }

  private view(length: number): DataView {
    if (length > this.len - this.pos) throw new IpcError('invalid-payload');
    return new DataView(this.bytesView.buffer, this.bytesView.byteOffset + this.pos, length);
  }

  private readFixed32(): number {
    const view = this.view(4);
    const value = view.getUint32(0, true);
    this.pos += 4;
    return value;
  }

  private readFixed64(): bigint {
    const view = this.view(8);
    const value = view.getBigUint64(0, true);
    this.pos += 8;
    return value;
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
