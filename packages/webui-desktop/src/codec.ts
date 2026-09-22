// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { IpcError, ipcError } from './errors.js';
import { IpcFrame } from './generated/webui_desktop.js';
import type { ByteLedger } from './budget.js';
import type { IpcLimits } from './limits.js';
import type { MessageCodec } from './types.js';

/** Validate first, reserve transient writer space, then encode the full frame. */
export function encodePayloadFrame<T>(codec: MessageCodec<T>, value: unknown, message: IpcFrame, ledger: ByteLedger): Uint8Array {
  let release: (() => void) | undefined;
  try {
    codec.validate(value, ledger.limits);
    // Generated codecs grow binary writers geometrically and finish with an
    // exact-sized copy. Reserve their bounded transient working set first.
    const upper = ledger.limits.maxFrameBytes + ledger.limits.maxCollectionEntriesPerMessage * 16 + 256;
    release = ledger.reserve(upper * 4);
    const payload = codec.encode(value as T);
    if (payload.byteLength > ledger.limits.maxFrameBytes) throw new IpcError('payload-too-large');
    message.body = { $case: 'payload', value: payload };
    const bytes = IpcFrame.encode(message).finish();
    if (bytes.byteLength > ledger.limits.maxFrameBytes) throw new IpcError('payload-too-large');
    return bytes;
  } catch (error) { throw ipcError(error, 'invalid-payload'); }
  finally { release?.(); }
}

/** Guard binary lengths and collections before allocating decoded objects. */
export function decodePayload<T>(codec: MessageCodec<T>, bytes: Uint8Array, limits: IpcLimits): T {
  try {
    codec.validateBytes(bytes, limits);
    const value = codec.decode(bytes);
    codec.validate(value, limits);
    return value;
  } catch (error) { throw ipcError(error, 'invalid-payload'); }
}
