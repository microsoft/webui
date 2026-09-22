// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { IpcError } from './errors.js';
import type { IpcLimits } from './limits.js';
import type { BinaryReceiver } from './types.js';

/** Shared document-realm ledger; retiring an epoch never resets task credits. */
export class ByteLedger {
  private state: { input: number; retained: number; limits: IpcLimits };
  constructor(limits: IpcLimits) { this.state = { input: 0, retained: 0, limits }; }
  get input(): number { return this.state.input; }
  set input(value: number) { this.state.input = value; }
  get retained(): number { return this.state.retained; }
  set retained(value: number) { this.state.retained = value; }
  get limits(): IpcLimits { return this.state.limits; }
  set limits(value: IpcLimits) { this.state.limits = value; }
  shareWith(previous: ByteLedger): void {
    if (this.state === previous.state) return;
    if (this.input || this.retained) throw new IpcError('invalid-frame', 'Cannot replace an active byte ledger.');
    this.state = previous.state;
  }
  reserve(bytes: number): () => void {
    if (!Number.isSafeInteger(bytes) || bytes < 0 || this.retained + bytes > this.limits.maxRetainedBytesPerFrame) {
      throw new IpcError('overloaded');
    }
    this.retained += bytes;
    let active = true;
    return () => { if (active) { active = false; this.retained -= bytes; } };
  }
  reserveInput(bytes: number): InputCredit {
    return new InputCredit(this, bytes);
  }
}

/** Transferable input credit, kept until all admitted callbacks actually finish. */
export class InputCredit {
  private releases: (() => void)[] = [];
  private active = true;
  capacity = 0;
  constructor(readonly owner: ByteLedger, bytes: number) { this.grow(bytes); }
  grow(bytes: number): void {
    if (!this.active || this.owner.input + bytes > this.owner.limits.maxAdmittedInputBytesPerFrame) throw new IpcError('overloaded');
    const release = this.owner.reserve(bytes);
    this.owner.input += bytes;
    this.capacity += bytes;
    this.releases.push(release);
  }
  release(): void {
    if (!this.active) return;
    this.active = false;
    this.owner.input -= this.capacity;
    for (const release of this.releases) release();
    this.releases = [];
  }
  owns(bytes: Uint8Array, ledger: ByteLedger): boolean {
    return this.active && this.owner === ledger && this.capacity >= bytes.buffer.byteLength;
  }
}

/** Optional private transport optimization; the public BinaryReceiver stays minimal. */
export interface BudgetReceiver extends BinaryReceiver {
  readonly byteLedger: ByteLedger;
  receiveReserved(bytes: Uint8Array, credit: InputCredit): Promise<void>;
}
export function hasLedger(receiver: BinaryReceiver): receiver is BudgetReceiver {
  return 'byteLedger' in receiver && 'receiveReserved' in receiver;
}
