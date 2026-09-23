// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createConnection } from './connection.js';
import { IpcError } from './errors.js';
import type {
  CallOptions, DesktopConnection, EventDescriptor, Hello, IpcTransport, MessageCodec,
  MethodDescriptor, RequestContext, RpcDescriptor, Subscription,
} from './types.js';

/** Erased implementation seam used only behind generated typed wrappers. */
export interface MethodDefinition {
  readonly id: number;
  readonly name: string;
  readonly receiver: 'host' | 'renderer';
  readonly kind: 'rpc' | 'notification';
  readonly developmentOnly: boolean;
  readonly request: MessageCodec<any>;
  readonly response?: MessageCodec<any>;
}
export interface ConnectionSchema { readonly hello: Hello; readonly methods: readonly MethodDefinition[] }
type Handlers = ReadonlyMap<number, (value: any, context: RequestContext) => any>;
export interface RuntimeConnection extends DesktopConnection {
  call<T>(id: number, value: unknown, options?: CallOptions): Promise<T>;
  notify(id: number, value: unknown): Promise<void>;
  subscribe(id: number, callback: (value: any) => void | Promise<void>): Subscription;
  register(handlers: Handlers): Subscription;
}

/** Generated applications use this seam; application authors use their typed proxies. */
export async function connect(
  transport: IpcTransport,
  schema: ConnectionSchema,
  options: { handlers?: Handlers; onError?: (error: IpcError) => void } = {},
): Promise<RuntimeConnection> {
  const methods = new Map<number, MethodDescriptor>();
  for (const method of schema.methods) {
    if (methods.has(method.id) || (method.kind === 'rpc' && !method.response)) throw new IpcError('invalid-payload');
    methods.set(method.id, method as MethodDescriptor);
  }
  const get = (id: number, kind: string, receiver: string): MethodDescriptor => {
    const method = methods.get(id);
    if (!method || method.kind !== kind || method.receiver !== receiver) throw new IpcError('unknown-method');
    return method;
  };
  const registered = new Set<number>();
  let register!: (handlers: Handlers) => Subscription;
  const connection = await createConnection({ ...schema.hello, methods: [...methods.values()] }, transport, {
    ...(options.onError ? { onError: options.onError } : {}),
    setup(connection) {
      register = handlers => {
        // Preflight all IDs before mutating the connection.
        for (const id of handlers.keys()) {
          get(id, 'rpc', 'renderer');
          if (registered.has(id)) throw new IpcError('invalid-payload', 'Duplicate renderer registration.');
        }
        const subscriptions: Subscription[] = [];
        const ids: number[] = [];
        for (const [id, handler] of handlers) {
          subscriptions.push(connection.handle(get(id, 'rpc', 'renderer') as RpcDescriptor<any, any, 'renderer'>, handler));
          registered.add(id);
          ids.push(id);
        }
        let active = true;
        return { close() {
          if (!active) return;
          active = false;
          for (const subscription of subscriptions) subscription.close();
          for (const id of ids) registered.delete(id);
        } };
      };
      if (options.handlers) register(options.handlers);
    },
  });
  return {
    closed: connection.closed,
    close: () => connection.close(),
    call: <T>(id: number, value: unknown, callOptions?: CallOptions) =>
      connection.call(get(id, 'rpc', 'host') as RpcDescriptor<unknown, T, 'host'>, value, callOptions),
    notify: (id, value) => connection.notify(get(id, 'notification', 'host') as EventDescriptor<unknown, 'host'>, value),
    subscribe: (id, callback) => connection.subscribe(get(id, 'notification', 'renderer') as EventDescriptor<any, 'renderer'>, callback),
    register,
  };
}
