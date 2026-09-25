// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { IpcError } from './errors.js';

export const defaultLimits = Object.freeze({
  maxFrameBytes: 1048576,
  maxNativeControlBytes: 4096,
  maxPendingCallsPerDirection: 64,
  maxOutstandingNotificationsPerDirection: 64,
  maxQueuedFramesPerDirection: 128,
  maxQueuedBytesPerDirection: 8388608,
  maxAdmittedInputBytesPerFrame: 8388608,
  maxRetainedBytesPerFrame: 16777216,
  maxWorkerTasksPerFrameIncludingRetiredDocuments: 128,
  maxCallbacksPerEvent: 16,
  maxCallbacksPerDocument: 128,
  maxCallbackTasksPerDocument: 128,
  maxCollectionEntriesPerMessage: 16384,
  maxSchemaDepth: 16,
  maxReadinessWaiters: 16,
  defaultTimeoutMs: 30000,
  maxTimeoutMs: 300000,
  notificationAcceptTimeoutMs: 30000,
  handshakeTimeoutMs: 5000,
  maxErrorTextBytesTotal: 2048,
  reservedControlFramesPerDirection: 128,
});
export type IpcLimits = { -readonly [K in keyof typeof defaultLimits]: number };

export function validateLimits(value: IpcLimits): void {
  for (const key of Object.keys(defaultLimits) as (keyof IpcLimits)[]) {
    if (!Number.isSafeInteger(value[key]) || value[key] <= 0) {
      throw new IpcError('invalid-frame', `Invalid IPC limit: ${key}`);
    }
  }
  if (value.maxFrameBytes > 16 * 1048576 || value.maxNativeControlBytes > 65536 ||
      value.maxPendingCallsPerDirection > 4096 || value.maxCallbacksPerDocument > 1024 ||
      value.maxQueuedBytesPerDirection > 64 * 1048576 ||
      value.maxFrameBytes > value.maxQueuedBytesPerDirection ||
      value.defaultTimeoutMs > value.maxTimeoutMs || value.maxTimeoutMs > 300000 ||
      value.maxOutstandingNotificationsPerDirection > 4096 ||
      value.maxCallbacksPerEvent > value.maxCallbacksPerDocument ||
      value.maxSchemaDepth > 16 || value.maxReadinessWaiters > 16 ||
      value.notificationAcceptTimeoutMs > value.maxTimeoutMs ||
      value.maxWorkerTasksPerFrameIncludingRetiredDocuments > 4096 ||
      value.maxCallbackTasksPerDocument > 4096 || value.maxQueuedFramesPerDirection > 8192 ||
      value.reservedControlFramesPerDirection > 8192 ||
      value.maxCollectionEntriesPerMessage > 1048576 || value.maxErrorTextBytesTotal > 2048 ||
      value.handshakeTimeoutMs > 5000 ||
      value.maxFrameBytes < value.maxErrorTextBytesTotal + 128) {
    throw new IpcError('invalid-frame', 'Inconsistent IPC limits.');
  }
  if (value.maxAdmittedInputBytesPerFrame > 64 * 1048576 ||
      value.maxRetainedBytesPerFrame > 128 * 1048576 ||
      value.maxAdmittedInputBytesPerFrame < value.maxFrameBytes ||
      value.maxRetainedBytesPerFrame < value.maxAdmittedInputBytesPerFrame) {
    throw new IpcError('invalid-frame', 'Inconsistent IPC byte budgets.');
  }
}
