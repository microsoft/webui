// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { IpcError, IpcErrorCode } from './errors.js';
import type { IpcLimits } from './limits.js';

export interface Subscription { close(): void }
export interface CallOptions { signal?: AbortSignal; timeoutMs?: number }
export interface RequestContext { readonly signal: AbortSignal; readonly deadlineMs: number }
export interface Hello {
  wireVersion: 2;
  contractName: string;
  contractMajor: number;
  schemaHash: string;
}
export interface SessionInfo { generation: string; token: string; limits: IpcLimits }
export interface BinaryReceiver {
  receive(frame: Uint8Array): Promise<void>;
  closed(error: IpcError): void;
}
export interface IpcTransport {
  start(hello: Hello, receiver: BinaryReceiver): Promise<SessionInfo>;
  send(frame: Uint8Array): Promise<void>;
  close(): void;
}
export type NativeControl =
  | { kind: 'ready'; generation: string }
  | { kind: 'closed'; generation: string; code: IpcErrorCode };
export interface DocumentActivation { navigation: string; documentNonce: string; challenge: string }
export interface NativeIpcBootstrap {
  readonly documentNonce: string;
  activate(activation: DocumentActivation): boolean;
  hello(hello: Hello): Promise<SessionInfo>;
  subscribeControl(listener: (control: NativeControl) => void): Subscription;
  disconnect(generation: string, token: string): void;
}
declare global {
  interface Window { readonly __webuiDesktopIpcV2?: NativeIpcBootstrap }
}

/** Codecs wrap generated protobuf encode/decode and generated validation. */
export interface MessageCodec<T> {
  encode(value: T): Uint8Array;
  decode(bytes: Uint8Array): T;
  validate(value: unknown, limits: IpcLimits): void;
  /** Must guard wire lengths, nesting and collection counts before codec decode. */
  validateBytes(bytes: Uint8Array, limits: IpcLimits): void;
}
export interface RpcDescriptor<Q, R, E extends Endpoint = Endpoint> {
  readonly id: number;
  readonly kind: 'rpc';
  readonly receiver: E;
  readonly request: MessageCodec<Q>;
  readonly response: MessageCodec<R>;
}
export interface EventDescriptor<T, E extends Endpoint = Endpoint> {
  readonly id: number;
  readonly kind: 'notification';
  readonly receiver: E;
  readonly request: MessageCodec<T>;
}
export type Endpoint = 'host' | 'renderer';
// The erased descriptor is only used for lookup, never for application calls.
export type MethodDescriptor = RpcDescriptor<any, any> | EventDescriptor<any>;
export interface IpcSchema extends Hello { readonly methods: readonly MethodDescriptor[] }
export interface DesktopConnection { close(): void; readonly closed: Promise<IpcError> }
