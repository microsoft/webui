// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export { Connection, createConnection, type ConnectionOptions } from './connection.js';
export { createDesktopTransport, type DesktopTransportOptions } from './transport.js';
export { IpcError, type IpcErrorCode } from './errors.js';
export { defaultLimits, type IpcLimits } from './limits.js';
export { connect, type ConnectionSchema, type MethodDefinition, type RuntimeConnection } from './connect.js';
export { validateMessage, validateValue, type FieldShape, type MessageShape } from './validation.js';
export { BoundaryReader, PayloadWriter, readDelimited, readUint32, readUint64, skipField } from './boundary.js';
export type {
  BinaryReceiver, CallOptions, DesktopConnection, DocumentActivation, Endpoint, EventDescriptor, Hello,
  IpcSchema, IpcTransport, MessageCodec, MethodDescriptor, NativeControl,
  NativeIpcBootstrap, RequestContext, RpcDescriptor, SessionInfo, Subscription,
} from './types.js';
