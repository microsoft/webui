// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { IpcError, ipcError } from './errors.js';
import type { IpcLimits } from './limits.js';
import type { IpcTransport } from './types.js';
import type { ByteLedger } from './budget.js';

interface Entry {
  bytes: Uint8Array;
  control: boolean;
  prepare?: (bytes: Uint8Array) => Uint8Array | undefined;
  id?: bigint;
  release(): void;
  resolve(): void;
  reject(error: IpcError): void;
}
export interface ControlCredit { readonly owner: OutputQueue; claim(): boolean; release(): void }

/** One bounded serialized POST queue, with a separate control reserve. */
export class OutputQueue {
  private entries: Entry[] = [];
  private bytes = 0;
  private data = 0;
  private controls = 0;
  private reserved = 0;
  private running = false;
  private failure?: IpcError;
  constructor(private transport: IpcTransport, private limits: IpcLimits, private ledger: ByteLedger) {}

  reserveControl(): ControlCredit {
    if (this.failure) throw this.failure;
    if (this.controls + this.reserved >= this.limits.reservedControlFramesPerDirection) throw new IpcError('overloaded');
    this.reserved++;
    let active = true;
    const release = () => {
      if (!active) return false;
      active = false;
      if (!this.failure) this.reserved--;
      return true;
    };
    return { owner: this, claim: release, release };
  }

  send(bytes: Uint8Array, control: boolean, prepare?: Entry['prepare'], id?: bigint, credit?: ControlCredit): Promise<void> {
    if (this.failure) return Promise.reject(this.failure);
    if (bytes.byteLength > this.limits.maxFrameBytes) return Promise.reject(new IpcError('payload-too-large'));
    const reserved = credit?.owner === this && credit.claim();
    if (control ? !reserved && this.controls + this.reserved >= this.limits.reservedControlFramesPerDirection
      : this.data >= this.limits.maxQueuedFramesPerDirection ||
        this.bytes + bytes.byteLength > this.limits.maxQueuedBytesPerDirection) {
      return Promise.reject(new IpcError('overloaded'));
    }
    // Control frames are bounded errors/ACKs/CANCELs, never user RESULT payloads.
    if (control && bytes.byteLength > this.limits.maxErrorTextBytesTotal + 128) {
      return Promise.reject(new IpcError('payload-too-large'));
    }
    let release: () => void;
    try { release = control ? () => {} : this.ledger.reserve(bytes.buffer.byteLength); }
    catch (error) { return Promise.reject(ipcError(error)); }
    if (control) this.controls++; else { this.data++; this.bytes += bytes.byteLength; }
    return new Promise<void>((resolve, reject) => {
      const entry: Entry = { bytes, control, resolve, reject, release };
      if (prepare) entry.prepare = prepare;
      if (id !== undefined) entry.id = id;
      this.entries.push(entry);
      void this.pump();
    });
  }

  /** Drop unsent work immediately, releasing bytes even behind a blocked POST. */
  remove(id: bigint): void {
    const index = this.entries.findIndex((entry, index) => entry.id === id && (!this.running || index !== 0));
    if (index < 0) return;
    const [entry] = this.entries.splice(index, 1);
    if (!entry) return;
    this.release(entry);
    entry.resolve();
  }

  close(error: IpcError): void {
    this.failure ??= error;
    for (let i = 0; i < this.entries.length; i++) {
      const entry = this.entries[i]!;
      entry.reject(error);
      if (!this.running || i !== 0) entry.release();
    }
    this.entries = [];
    this.bytes = this.data = this.controls = this.reserved = 0;
  }

  private async pump(): Promise<void> {
    if (this.running) return;
    this.running = true;
    try {
      while (this.entries.length && !this.failure) {
        const entry = this.entries[0]!;
        let releasePrepared: (() => void) | undefined;
        try {
          let bytes: Uint8Array | undefined;
          if (entry.prepare) {
            const releaseScratch = this.ledger.reserve(entry.bytes.byteLength * 3 + 256);
            try { bytes = entry.prepare(entry.bytes); } finally { releaseScratch(); }
          } else bytes = entry.bytes;
          if (bytes && bytes !== entry.bytes && !entry.control) releasePrepared = this.ledger.reserve(bytes.buffer.byteLength);
          if (bytes) await this.transport.send(bytes);
          entry.resolve();
        } catch (error) { entry.reject(ipcError(error)); }
        finally { releasePrepared?.(); }
        if (this.failure) { entry.release(); break; }
        this.entries.shift();
        this.release(entry);
      }
    } finally { this.running = false; }
  }

  private release(entry: Entry): void {
    entry.release();
    if (entry.control) this.controls--;
    else { this.data--; this.bytes -= entry.bytes.byteLength; }
  }
}
