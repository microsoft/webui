// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { IpcError, errorCode } from './errors.js';
import { IpcFrame, Kind, type WireError } from './envelope.js';
import type { IpcLimits } from './limits.js';

/** Current envelope wire version (see `webui_desktop::ipc::IPC_VERSION`). The
 * renderer runtime and the Rust host ship in the same binary and are never
 * independently versioned, so a stale build fails closed here rather than
 * misparsing bytes in the new layout. */
export const IPC_VERSION = 3;

export { IpcFrame, Kind };
export function frame(generation: bigint, id: bigint, kind: Kind): IpcFrame {
  return { version: IPC_VERSION, generation, id, kind, methodId: 0, timeoutMs: 0 };
}

/**
 * Decode and semantically validate a v3 envelope. The fixed layout has no
 * ambiguity to police (no duplicate/unknown fields, no wire-type confusion),
 * so `IpcFrame.decode` alone both parses and structurally validates the
 * bytes; only the envelope's own semantic invariants are checked here.
 */
export function decodeFrame(bytes: Uint8Array, limits: IpcLimits): IpcFrame {
  if (bytes.byteLength > limits.maxFrameBytes) throw new IpcError('payload-too-large');
  try {
    const value = IpcFrame.decode(bytes);
    if (value.version !== IPC_VERSION) throw new IpcError('unsupported-version');
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
    if (error instanceof IpcError) throw error;
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
