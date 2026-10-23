// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { PartialResponse } from './cache.js';

/** Partial metadata that may still own an unread response tail. */
export interface StreamingPartialResponse extends PartialResponse {
  readonly _deferredStream?: true;
  inventory?: string;
  styleInventory?: string;
  buildId?: string;
}

export interface DeferredStreamControl {
  cancel(): Promise<void>;
  start(): void;
}

const deferredStreams = new WeakMap<PartialResponse, DeferredStreamControl>();

export function attachDeferredStream(
  data: StreamingPartialResponse,
  control: DeferredStreamControl,
): void {
  deferredStreams.set(data, control);
  Object.defineProperty(data, '_deferredStream', { configurable: true, value: true });
}

export function releaseDeferredStream(data: PartialResponse): void {
  deferredStreams.delete(data);
  Reflect.deleteProperty(data, '_deferredStream');
}

/** Whether a partial response still owns deferred state delivery. */
export function hasDeferredStream(data: PartialResponse): data is StreamingPartialResponse {
  return deferredStreams.has(data);
}

/** Begin deferred state delivery only after the consumer commits its UI. */
export function startDeferredStream(data: PartialResponse): void {
  deferredStreams.get(data)?.start();
}

/** Cancel an abandoned partial without importing a stream decoder. */
export async function cancelDeferredStream(data: PartialResponse): Promise<void> {
  const control = deferredStreams.get(data);
  releaseDeferredStream(data);
  await control?.cancel();
}
