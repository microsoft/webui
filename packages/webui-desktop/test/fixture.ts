// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { BoundaryReader, PayloadWriter, readDelimited, readUint64, skipField } from '../src/index.js';

export interface Item {
  id: bigint;
  signed: bigint;
  data: Uint8Array;
}

export interface Empty {}

export interface FinishedWriter { finish(): Uint8Array }

export const Item = {
  encode(message: Item): FinishedWriter {
    const writer = new PayloadWriter();
    if (message.id !== 0n) writer.uint64(1, message.id);
    if (message.signed !== 0n) writer.sint64(2, message.signed);
    if (message.data.byteLength !== 0) writer.bytesField(3, message.data);
    return writer;
  },

  decode(bytes: Uint8Array): Item {
    const reader = new BoundaryReader(bytes);
    const message: Item = { id: 0n, signed: 0n, data: new Uint8Array(0) };
    while (reader.pos < reader.len) {
      const [number, type] = reader.tag();
      switch (number) {
        case 1:
          message.id = readUint64(reader);
          break;
        case 2: {
          const raw = readUint64(reader);
          message.signed = (raw >> 1n) ^ -(raw & 1n);
          break;
        }
        case 3:
          message.data = readDelimited(reader);
          break;
        default:
          skipField(reader, type);
      }
    }
    return message;
  },
};

export const Empty = {
  encode(_: Empty): FinishedWriter { return new PayloadWriter(); },
  decode(bytes: Uint8Array): Empty {
    const reader = new BoundaryReader(bytes);
    while (reader.pos < reader.len) {
      const [, type] = reader.tag();
      skipField(reader, type);
    }
    return {};
  },
};
