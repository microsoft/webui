// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { IpcError, errorCode } from './errors.js';
import { IpcFrame, Kind, type WireError } from './generated/webui_desktop.js';
import type { IpcLimits } from './limits.js';
import { BoundaryReader, readDelimited, readUint32, skipField } from './boundary.js';
import { validateMessage, type MessageShape } from './validation.js';

const errorShape: readonly MessageShape[] = [{ fields: [1, 2, 3, 4].map(number => ({
  number, name: '', kind: 'string', repeated: false, optional: false, mapKey: false,
})) }];

export { IpcFrame, Kind };
export function frame(generation: bigint, id: bigint, kind: Kind): IpcFrame {
  return { version: 2, generation, id, kind, methodId: 0, timeoutMs: 0 };
}

/** Reject ambiguous envelopes and groups before the mature protobuf decoder. */
export function decodeFrame(bytes: Uint8Array, limits: IpcLimits): IpcFrame {
  if (bytes.byteLength > limits.maxFrameBytes) throw new IpcError('payload-too-large');
  try {
    const reader = new BoundaryReader(bytes);
    let seen = 0;
    while (reader.pos < reader.len) {
      const [field, wire] = reader.tag();
      if (!field || wire === 3 || wire === 4 || wire > 5) throw new IpcError('invalid-frame');
      if (field <= 8) {
        const expected = field === 2 || field === 3 ? 1 : field >= 7 ? 2 : 0;
        if (wire !== expected || (seen & (1 << field)) ||
            (field >= 7 && (seen & ((1 << 7) | (1 << 8))))) throw new IpcError('invalid-frame');
        seen |= 1 << field;
      }
      if (field === 8) validateMessage(readDelimited(reader), 0, errorShape, limits);
      else if (field <= 6 && wire === 0) readUint32(reader);
      else skipField(reader, wire);
    }
    const value = IpcFrame.decode(bytes);
    if (value.version !== 2) throw new IpcError('unsupported-version');
    if (!value.id || !value.generation || value.kind < 1 || value.kind > 6) throw new IpcError('invalid-frame');
    const invocation = value.kind === Kind.REQUEST || value.kind === Kind.NOTIFY;
    if (invocation ? !value.methodId : value.methodId !== 0 || value.timeoutMs !== 0) throw new IpcError('invalid-frame');
    if (value.kind === Kind.REQUEST ? !value.timeoutMs || value.timeoutMs > limits.maxTimeoutMs
      : value.timeoutMs !== 0) throw new IpcError('invalid-frame');
    const body = value.kind === Kind.ERROR ? 'error'
      : value.kind === Kind.REQUEST || value.kind === Kind.NOTIFY || value.kind === Kind.RESULT ? 'payload' : undefined;
    if (value.body?.$case !== body) throw new IpcError('invalid-frame');
    if (value.body?.$case === 'error') {
      const e = value.body.value;
      if (new TextEncoder().encode(e.code + e.message + e.help + e.applicationCode).length > limits.maxErrorTextBytesTotal) {
        throw new IpcError('invalid-frame');
      }
    }
    return value;
  } catch (error) {
    if (error instanceof IpcError && error.code !== 'invalid-payload') throw error;
    throw new IpcError('invalid-frame');
  }
}

export function fromWire(error: WireError): IpcError {
  return new IpcError(errorCode(error.code), error.message, error.help, error.applicationCode || undefined);
}

export function toWire(error: IpcError): WireError {
  // Fixed SDK diagnostics prevent exception text, credentials and stacks escaping.
  return { code: error.code, message: error.code, help: 'Check the connection and contract.', applicationCode: '' };
}
