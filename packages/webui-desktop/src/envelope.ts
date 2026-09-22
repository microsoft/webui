// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Fixed-layout binary codec for the framework-owned IPC envelope.
 *
 * `IpcFrame`/`WireError` are exchanged only between this SDK's renderer runtime
 * and its own Rust host, both shipped in the same packaged binary and always
 * versioned together - so there is no cross-binary schema evolution to
 * support. A fixed byte layout replaces the previously protobuf-encoded
 * envelope: a far smaller runtime bundle, faster encode/decode, and a single
 * decode pass instead of a lenient-format pre-scan plus a full parse.
 * Application-defined message payloads remain arbitrary opaque bytes here and
 * are still validated generically (see `validation.ts`). See `DESIGN.md` for
 * the authoritative byte layout.
 */

import { IpcError } from './errors.js';

export enum Kind {
  KIND_UNSPECIFIED = 0,
  REQUEST = 1,
  RESULT = 2,
  ERROR = 3,
  NOTIFY = 4,
  ACCEPT = 5,
  CANCEL = 6,
}

export interface IpcFrame {
  version: number;
  generation: bigint;
  id: bigint;
  kind: Kind;
  methodId: number;
  timeoutMs: number;
  body?: { $case: 'payload'; value: Uint8Array } | { $case: 'error'; value: WireError } | undefined;
}

export interface WireError {
  code: string;
  message: string;
  help: string;
  applicationCode: string;
}

export interface MessageFns<T> {
  encode(message: T): { finish(): Uint8Array<ArrayBuffer> };
  decode(bytes: Uint8Array): T;
}

const HEADER_LEN = 30;
const BODY_TAG_NONE = 0;
const BODY_TAG_PAYLOAD = 1;
const BODY_TAG_ERROR = 2;

/** A cursor over an input buffer; every read is bounds-checked. */
class Cursor {
  pos = 0;
  constructor(readonly view: DataView, readonly bytes: Uint8Array) {}

  private take(len: number): number {
    if (this.pos + len > this.bytes.length) throw new IpcError('invalid-frame');
    const start = this.pos;
    this.pos += len;
    return start;
  }

  u8(): number {
    return this.view.getUint8(this.take(1));
  }
  u32(): number {
    return this.view.getUint32(this.take(4), true);
  }
  u64(): bigint {
    return this.view.getBigUint64(this.take(8), true);
  }
  bytes_(len: number): Uint8Array {
    const start = this.take(len);
    return this.bytes.subarray(start, start + len);
  }
  /** Reads a `[u32 length][bytes]` block, bounds-checked against what remains. */
  delimited(): Uint8Array {
    const len = this.u32();
    if (len > this.bytes.length - this.pos) throw new IpcError('invalid-frame');
    return this.bytes_(len);
  }
  string(): string {
    return new TextDecoder('utf-8', { fatal: true }).decode(this.delimited());
  }
  atEnd(): boolean {
    return this.pos === this.bytes.length;
  }
}

class Writer {
  private chunks: Uint8Array[] = [];

  u8(value: number): void {
    this.chunks.push(Uint8Array.of(value & 0xff));
  }
  u32(value: number): void {
    const buf = new Uint8Array(4);
    new DataView(buf.buffer).setUint32(0, value, true);
    this.chunks.push(buf);
  }
  u64(value: bigint): void {
    const buf = new Uint8Array(8);
    new DataView(buf.buffer).setBigUint64(0, value, true);
    this.chunks.push(buf);
  }
  raw(bytes: Uint8Array): void {
    this.chunks.push(bytes);
  }
  delimited(bytes: Uint8Array): void {
    this.u32(bytes.length);
    this.raw(bytes);
  }
  string(value: string): void {
    this.delimited(new TextEncoder().encode(value));
  }
  finish(): Uint8Array<ArrayBuffer> {
    let total = 0;
    for (const chunk of this.chunks) total += chunk.length;
    const out = new Uint8Array(total);
    let offset = 0;
    for (const chunk of this.chunks) {
      out.set(chunk, offset);
      offset += chunk.length;
    }
    return out;
  }
}

function writeError(writer: Writer, error: WireError): void {
  writer.string(error.code);
  writer.string(error.message);
  writer.string(error.help);
  writer.string(error.applicationCode);
}

function readError(cursor: Cursor): WireError {
  return {
    code: cursor.string(),
    message: cursor.string(),
    help: cursor.string(),
    applicationCode: cursor.string(),
  };
}

export const WireError: MessageFns<WireError> = {
  /** Encodes the standalone (headerless) four-string form. */
  encode(message: WireError): { finish(): Uint8Array<ArrayBuffer> } {
    const writer = new Writer();
    writeError(writer, message);
    return writer;
  },
  /** Decodes the standalone (headerless) four-string form, rejecting trailing bytes. */
  decode(bytes: Uint8Array): WireError {
    const cursor = new Cursor(new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength), bytes);
    const value = readError(cursor);
    if (!cursor.atEnd()) throw new IpcError('invalid-frame');
    return value;
  },
};

export const IpcFrame: MessageFns<IpcFrame> = {
  encode(message: IpcFrame): { finish(): Uint8Array<ArrayBuffer> } {
    const writer = new Writer();
    writer.u32(message.version);
    writer.u64(message.generation);
    writer.u64(message.id);
    // An unspecified/out-of-range kind is always rejected by the caller's
    // semantic validation, so clamping here can never produce a valid-looking
    // frame.
    writer.u8(message.kind >= 0 && message.kind <= 6 ? message.kind : 0);
    writer.u32(message.methodId);
    writer.u32(message.timeoutMs);
    switch (message.body?.$case) {
      case undefined:
        writer.u8(BODY_TAG_NONE);
        break;
      case 'payload':
        writer.u8(BODY_TAG_PAYLOAD);
        writer.delimited(message.body.value);
        break;
      case 'error':
        writer.u8(BODY_TAG_ERROR);
        writeError(writer, message.body.value);
        break;
    }
    return writer;
  },

  /**
   * Decodes a full envelope, rejecting truncated, malformed, or trailing
   * bytes. The fixed layout has no ambiguity to police (no duplicate/unknown
   * fields, no wire-type confusion), so a single pass both parses and
   * validates structural well-formedness.
   */
  decode(bytes: Uint8Array): IpcFrame {
    if (bytes.length < HEADER_LEN) throw new IpcError('invalid-frame');
    const cursor = new Cursor(new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength), bytes);
    const version = cursor.u32();
    const generation = cursor.u64();
    const id = cursor.u64();
    const kind = cursor.u8() as Kind;
    const methodId = cursor.u32();
    const timeoutMs = cursor.u32();
    const bodyTag = cursor.u8();
    let body: IpcFrame['body'];
    switch (bodyTag) {
      case BODY_TAG_NONE:
        body = undefined;
        break;
      case BODY_TAG_PAYLOAD:
        body = { $case: 'payload', value: cursor.delimited() };
        break;
      case BODY_TAG_ERROR:
        body = { $case: 'error', value: readError(cursor) };
        break;
      default:
        throw new IpcError('invalid-frame');
    }
    if (!cursor.atEnd()) throw new IpcError('invalid-frame');
    return { version, generation, id, kind, methodId, timeoutMs, body };
  },
};
