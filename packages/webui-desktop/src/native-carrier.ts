// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { ByteLedger, type InputCredit } from './budget.js';
import { IpcError, errorCode, ipcError } from './errors.js';
import { decodeFrame, Kind } from './framing.js';
import type { NativeDataLane, SessionInfo, Subscription } from './types.js';

/** 24 KiB binary => 32 KiB base64 per native message, independent of the 4 KiB control lane. */
const CHUNK = 24 * 1024;
const ENCODED_CHUNK = 4 * (CHUNK / 3);
const VERSION = 1;
type Operation = 'send' | 'receive';
interface Pending {
  callId: string;
  operation: Operation;
  resolve(value: Record<string, unknown>): void;
  reject(error: IpcError): void;
  timer: ReturnType<typeof setTimeout>;
}

function response(raw: unknown): Record<string, unknown> {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) throw new IpcError('invalid-frame');
  const message = raw as Record<string, unknown>;
  // Examine the bounded data field before processing it or serializing a host object.
  if (message.data !== undefined && (typeof message.data !== 'string' || message.data.length > ENCODED_CHUNK)) {
    throw new IpcError('payload-too-large');
  }
  return message;
}

function encodedLength(bytes: number): number { return 4 * Math.ceil(bytes / 3); }

/** One request in flight; correlated WK replies and WebView2 host messages share this path. */
export class NativeFrameCarrier {
  private subscription: Subscription;
  private pending: Pending | undefined;
  private failure?: IpcError;
  private nextCallId = 1;
  private sending = false;
  private reading = false;
  private idle: Promise<void> = Promise.resolve();
  constructor(private lane: NativeDataLane, private session: SessionInfo, private ledger: ByteLedger) {
    this.subscription = lane.subscribe(raw => this.receiveReply(raw));
  }

  close(error = new IpcError('closed')): void {
    if (this.failure) return;
    this.failure = error;
    this.subscription.close();
    const pending = this.pending;
    this.pending = undefined;
    if (pending) { clearTimeout(pending.timer); pending.reject(error); }
  }

  private receiveReply(raw: unknown, direct = false, expectedCallId?: string): void {
    if (!this.pending || this.failure) return;
    const pending = this.pending;
    if (expectedCallId !== undefined && pending.callId !== expectedCallId) return;
    try {
      const message = response(raw);
      if (message.kind !== 'ipcDataResult') throw new IpcError('invalid-frame');
      if (message.callId !== pending.callId) {
        if (direct) throw new IpcError('invalid-frame');
        return; // A late WebView2 host result must not settle a newer request.
      }
      if (message.version !== VERSION || message.generation !== this.session.generation ||
          message.operation !== pending.operation) throw new IpcError('unsupported-version');
      this.pending = undefined;
      clearTimeout(pending.timer);
      if (message.error !== undefined) {
        if (message.data !== undefined || message.empty !== undefined) throw new IpcError('invalid-frame');
        const error = message.error;
        if (!error || typeof error !== 'object' || Array.isArray(error)) throw new IpcError('invalid-frame');
        throw new IpcError(errorCode((error as Record<string, unknown>).code));
      }
      pending.resolve(message);
    } catch (error) {
      this.pending = undefined;
      clearTimeout(pending.timer);
      pending.reject(ipcError(error, 'invalid-frame'));
    }
  }

  private async exchange(operation: Operation, fields: Record<string, unknown>): Promise<Record<string, unknown>> {
    // Only one send and one receive can wait here. The native adapter keeps one
    // cursor per direction, never an unbounded queue of marshaled messages.
    const previous = this.idle;
    let release!: () => void;
    this.idle = new Promise(resolve => { release = resolve; });
    await previous;
    try { return await this.dispatch(operation, fields); }
    finally { release(); }
  }

  private dispatch(operation: Operation, fields: Record<string, unknown>): Promise<Record<string, unknown>> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.pending) return Promise.reject(new IpcError('overloaded'));
    if (this.nextCallId > Number.MAX_SAFE_INTEGER) return Promise.reject(new IpcError('overloaded'));
    const callId = String(this.nextCallId++);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (this.pending?.callId !== callId) return;
        this.pending = undefined;
        reject(new IpcError('deadline-exceeded'));
      }, Math.min(this.session.limits.handshakeTimeoutMs, 5000));
      this.pending = { callId, operation, resolve, reject, timer };
      try {
        const result = this.lane.postMessage({
          kind: 'ipcData', version: VERSION, callId, generation: this.session.generation,
          token: this.session.token, operation, ...fields,
        });
        // WK handler-with-reply returns a promise; WebView2 uses correlated host messages.
        if (result !== undefined) void Promise.resolve(result).then(value => this.receiveReply(value, true, callId), error => {
          if (this.pending?.callId !== callId) return;
          const pending = this.pending;
          this.pending = undefined;
          clearTimeout(timer);
          pending?.reject(ipcError(error));
        });
      } catch (error) {
        this.pending = undefined;
        clearTimeout(timer);
        reject(ipcError(error));
      }
    });
  }

  async send(bytes: Uint8Array): Promise<void> {
    if (this.sending) throw new IpcError('overloaded');
    if (!bytes.byteLength || bytes.byteLength > this.session.limits.maxFrameBytes) throw new IpcError('payload-too-large');
    this.sending = true;
    try {
      // The OutputQueue already owns the frame credit. Charge all transient JS
      // binary-string/base64/marshal storage before encoding each chunk.
      for (let offset = 0; offset < bytes.byteLength;) {
        const count = Math.min(CHUNK, bytes.byteLength - offset);
        const release = this.ledger.reserve(count * 2 + encodedLength(count) * 4);
        try {
          const chunk = bytes.subarray(offset, offset + count);
          let binary = '';
          for (let index = 0; index < count; index += 8192) {
            binary += String.fromCharCode(...chunk.subarray(index, index + 8192));
          }
          const reply = await this.exchange('send', {
            offset, totalBytes: bytes.byteLength, data: btoa(binary),
          });
          if (reply.nextOffset !== offset + count || reply.complete !== (offset + count === bytes.byteLength)) {
            throw new IpcError('invalid-frame');
          }
        } finally { release(); }
        offset += count;
      }
    } finally { this.sending = false; }
  }

  /** A receive request is also the ACK for the previous chunk; one frame per drain. */
  async read(): Promise<{ bytes: Uint8Array; credit?: InputCredit } | undefined> {
    if (this.reading) throw new IpcError('overloaded');
    this.reading = true;
    let credit: InputCredit | undefined;
    try {
      let bytes: Uint8Array | undefined;
      let offset = 0;
      const controlCapacity = this.session.limits.maxErrorTextBytesTotal + 128;
      while (true) {
        // The first pull is limited to the existing one-frame control reserve:
        // CANCEL/ERROR/ACCEPT can drain even if application input credits are
        // exhausted. All later, larger chunks require aggregate ledger credit.
        const maximum = offset === 0 ? controlCapacity : CHUNK;
        const release = offset === 0 ? () => {} : this.ledger.reserve(ENCODED_CHUNK * 4 + CHUNK * 2);
        let reply: Record<string, unknown>;
        try {
          reply = await this.exchange('receive', { offset, maxBytes: maximum });
          if (reply.empty !== undefined) {
            if (reply.empty !== true || offset !== 0 || reply.data !== undefined ||
                reply.totalBytes !== undefined || reply.offset !== undefined ||
                reply.nextOffset !== undefined || reply.complete !== undefined) throw new IpcError('invalid-frame');
            return undefined;
          }
          if (!Number.isSafeInteger(reply.totalBytes) || (reply.totalBytes as number) < 1 ||
              (reply.totalBytes as number) > this.session.limits.maxFrameBytes ||
              reply.offset !== offset || typeof reply.data !== 'string' ||
              reply.data.length === 0 || reply.data.length > encodedLength(maximum) ||
              reply.data.length % 4 !== 0) throw new IpcError('invalid-frame');
          const total = reply.totalBytes as number;
          if (!bytes) {
            if (total > controlCapacity) credit = this.ledger.reserveInput(total);
            bytes = new Uint8Array(total);
          } else if (total !== bytes.byteLength) throw new IpcError('invalid-frame');
          const decoded = atob(reply.data);
          if (decoded.length === 0 || decoded.length > maximum || offset + decoded.length > total ||
              encodedLength(decoded.length) !== reply.data.length ||
              reply.nextOffset !== offset + decoded.length ||
              reply.complete !== (offset + decoded.length === total)) throw new IpcError('invalid-frame');
          for (let i = 0; i < decoded.length; i++) bytes[offset + i] = decoded.charCodeAt(i);
          offset += decoded.length;
        } finally { release(); }
        if (offset === bytes.byteLength) {
          const decoded = decodeFrame(bytes, this.session.limits);
          if (!credit && (decoded.kind === Kind.REQUEST || decoded.kind === Kind.NOTIFY || decoded.kind === Kind.RESULT)) {
            credit = this.ledger.reserveInput(bytes.byteLength);
          }
          return { bytes, ...(credit ? { credit } : {}) };
        }
      }
    } catch (error) { credit?.release(); throw error; }
    finally { this.reading = false; }
  }
}
