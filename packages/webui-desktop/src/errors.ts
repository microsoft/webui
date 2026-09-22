// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export const errorCodes = [
  'invalid-frame', 'invalid-payload', 'payload-too-large', 'unsupported-version',
  'schema-mismatch', 'permission-denied', 'unknown-method', 'receiver-unavailable',
  'not-ready', 'overloaded', 'cancelled', 'deadline-exceeded', 'navigated', 'closed',
  'transport', 'handler',
] as const;
export type IpcErrorCode = typeof errorCodes[number];

/** A bounded, stable IPC failure, without application stacks on the wire. */
export class IpcError extends Error {
  constructor(
    readonly code: IpcErrorCode,
    message: string = code,
    readonly help = 'Check the connection and the generated contract.',
    readonly applicationCode?: string,
  ) {
    super(message);
    this.name = 'IpcError';
  }
}

export function errorCode(value: unknown): IpcErrorCode {
  return errorCodes.includes(value as IpcErrorCode) ? value as IpcErrorCode : 'transport';
}

export function ipcError(value: unknown, fallback: IpcErrorCode = 'transport'): IpcError {
  return value instanceof IpcError ? value : new IpcError(fallback);
}
