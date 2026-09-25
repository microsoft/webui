// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { IpcError, ipcError } from './errors.js';
import { decodeFrame, frame, fromWire, IpcFrame, Kind, toWire } from './framing.js';
import { defaultLimits, validateLimits, type IpcLimits } from './limits.js';
import { OutputQueue, type ControlCredit } from './queue.js';
import { ByteLedger, type BudgetReceiver, type InputCredit } from './budget.js';
import { decodePayload, encodePayloadFrame } from './codec.js';
import { projectHello } from './hello.js';
import type {
  CallOptions, DesktopConnection, EventDescriptor, IpcSchema, IpcTransport,
  MessageCodec, MethodDescriptor, RequestContext, RpcDescriptor, Subscription,
} from './types.js';

interface Pending {
  kind: 'rpc' | 'notification';
  codec?: MessageCodec<any>;
  resolve(value: any): void;
  reject(error: IpcError): void;
  cleanup(): void;
}
interface Callback { active: boolean; tail: Promise<void>; invoke?: (value: any) => void | Promise<void> }
interface Handler { invoke?: (value: any, context: RequestContext) => any }
interface Incoming { abort: AbortController; timer: ReturnType<typeof setTimeout>; control: ControlCredit }
export interface ConnectionOptions {
  onError?: (error: IpcError) => void;
  /** Invoked before transport.start, so handlers exist before admission. */
  setup?: (connection: Connection) => void;
}

/** Document-scoped, generated-descriptor-based application connection. */
export class Connection implements DesktopConnection {
  readonly closed: Promise<IpcError>;
  readonly stats = { staleReplies: 0, callbackErrors: 0, timeouts: 0, cancellations: 0 };
  private resolveClosed!: (error: IpcError) => void;
  private failure?: IpcError;
  private limits: IpcLimits = defaultLimits;
  private generation = 0n;
  private nextId = 1n;
  private lastIncoming = 0n;
  private pending = new Map<bigint, Pending>();
  private handlers = new Map<number, Handler>();
  private callbacks = new Map<number, Set<Callback>>();
  private incoming = new Map<bigint, Incoming>();
  private methods = new Map<number, MethodDescriptor>();
  private output?: OutputQueue;
  private callbackCount = 0;
  private callbackTasks = 0;
  private handlerTasks = 0;
  private ledger = new ByteLedger(defaultLimits);

  constructor(private schema: IpcSchema, private transport: IpcTransport, private options: ConnectionOptions) {
    this.options = { ...options };
    this.closed = new Promise(resolve => { this.resolveClosed = resolve; });
    for (const method of schema.methods) {
      if (!Number.isInteger(method.id) || method.id < 1024 || method.id > 0xffffffff || this.methods.has(method.id)) {
        throw new IpcError('invalid-payload', 'Invalid or duplicate method ID.');
      }
      this.methods.set(method.id, method);
    }
  }

  /** Internal startup used by createConnection. */
  async start(): Promise<void> {
    const early: { bytes: Uint8Array; credit: InputCredit }[] = [];
    let earlyBytes = 0;
    try {
      const setup = this.options.setup;
      delete this.options.setup;
      setup?.(this);
      const receiveReserved = async (bytes: Uint8Array, credit: InputCredit) => {
          if (this.failure) { credit.release(); return; }
          if (!credit.owns(bytes, this.ledger)) { credit.release(); this.fail(new IpcError('invalid-frame')); return; }
          if (!this.generation) {
            earlyBytes += bytes.byteLength;
            if (early.length >= defaultLimits.maxQueuedFramesPerDirection || earlyBytes > defaultLimits.maxQueuedBytesPerDirection) {
              this.fail(new IpcError('overloaded'));
              credit.release();
              return;
            }
            early.push({ bytes, credit });
          } else this.receive(bytes, credit);
      };
      const receiver: BudgetReceiver = {
        byteLedger: this.ledger,
        receiveReserved,
        receive: async bytes => {
          if (this.generation) { this.receive(bytes); return; }
          await receiveReserved(bytes, this.ledger.reserveInput(bytes.buffer.byteLength));
        },
        closed: error => this.fail(error),
      };
      const session = await this.transport.start(projectHello(this.schema), receiver);
      if (this.failure) throw this.failure;
      validateLimits(session.limits);
      this.limits = Object.freeze({ ...session.limits });
      this.ledger.limits = this.limits;
      if (this.ledger.input > this.limits.maxAdmittedInputBytesPerFrame ||
          this.ledger.retained > this.limits.maxRetainedBytesPerFrame) throw new IpcError('overloaded');
      if (this.callbackCount > this.limits.maxCallbacksPerDocument) throw new IpcError('overloaded');
      for (const callbacks of this.callbacks.values()) {
        if (callbacks.size > this.limits.maxCallbacksPerEvent) throw new IpcError('overloaded');
      }
      this.generation = BigInt(session.generation);
      if (this.generation <= 0n || this.generation > 0xffffffffffffffffn ||
          String(this.generation) !== session.generation) throw new IpcError('invalid-frame');
      this.output = new OutputQueue(this.transport, this.limits, this.ledger);
      while (early.length) {
        const entry = early.shift()!;
        this.receive(entry.bytes, entry.credit);
      }
      if (this.failure) throw this.failure;
    } catch (error) {
      for (const entry of early) entry.credit.release();
      this.fail(ipcError(error));
      throw this.failure;
    }
  }

  call<Q, R>(method: RpcDescriptor<Q, R, 'host'>, request: Q, options: CallOptions = {}): Promise<R> {
    return this.invoke(method, request, options) as Promise<R>;
  }
  notify<T>(method: EventDescriptor<T, 'host'>, payload: T): Promise<void> {
    return this.invoke(method, payload, {}) as Promise<void>;
  }
  handle<Q, R>(method: RpcDescriptor<Q, R, 'renderer'>, handler: (request: Q, context: RequestContext) => R | Promise<R>): Subscription {
    this.checkMethod(method, 'renderer');
    if (this.handlers.has(method.id)) throw new IpcError('invalid-payload', 'Handler already registered.');
    const entry: Handler = { invoke: handler };
    this.handlers.set(method.id, entry);
    return { close: () => {
      if (this.handlers.get(method.id) === entry) this.handlers.delete(method.id);
      delete entry.invoke;
    } };
  }
  subscribe<T>(method: EventDescriptor<T, 'renderer'>, callback: (payload: T) => void | Promise<void>): Subscription {
    this.checkMethod(method, 'renderer');
    const set = this.callbacks.get(method.id) ?? new Set<Callback>();
    if (set.size >= this.limits.maxCallbacksPerEvent || this.callbackCount >= this.limits.maxCallbacksPerDocument) {
      throw new IpcError('overloaded');
    }
    const entry: Callback = { active: true, tail: Promise.resolve(), invoke: callback };
    set.add(entry);
    this.callbacks.set(method.id, set);
    this.callbackCount++;
    return { close: () => {
      if (!entry.active) return;
      entry.active = false;
      delete entry.invoke;
      set.delete(entry);
      this.callbackCount--;
      if (!set.size) this.callbacks.delete(method.id);
    } };
  }
  close(): void { this.fail(new IpcError('closed')); }

  private checkMethod(method: MethodDescriptor, endpoint: 'host' | 'renderer'): void {
    if (this.failure) throw this.failure;
    if (this.methods.get(method.id) !== method || method.receiver !== endpoint) throw new IpcError('unknown-method');
  }
  private async invoke(method: MethodDescriptor, value: unknown, options: CallOptions): Promise<unknown> {
    const started = performance.now();
    this.checkMethod(method, 'host');
    if (!this.output) throw new IpcError('not-ready');
    const timeout = method.kind === 'notification' ? this.limits.notificationAcceptTimeoutMs
      : options.timeoutMs ?? this.limits.defaultTimeoutMs;
    if (!Number.isInteger(timeout) || timeout <= 0 || timeout > this.limits.maxTimeoutMs) throw new IpcError('invalid-payload');
    if (options.signal?.aborted) throw new IpcError('cancelled');
    let count = 0;
    for (const pending of this.pending.values()) if (pending.kind === method.kind) count++;
    const max = method.kind === 'rpc' ? this.limits.maxPendingCallsPerDirection : this.limits.maxOutstandingNotificationsPerDirection;
    if (count >= max) throw new IpcError('overloaded');
    if (this.nextId > 0xffffffffffffffffn) { this.close(); throw this.failure; }
    const id = this.nextId++;
    const message = frame(this.generation, id, method.kind === 'rpc' ? Kind.REQUEST : Kind.NOTIFY);
    message.methodId = method.id;
    message.timeoutMs = method.kind === 'rpc' ? timeout : 0;
    const encoded = encodePayloadFrame(method.request, value, message, this.ledger);
    const control = method.kind === 'rpc' ? this.output.reserveControl() : undefined;
    const deadline = started + timeout;
    return new Promise((resolve, reject) => {
      let sent = false;
      const cancel = (code: 'cancelled' | 'deadline-exceeded') => {
        if (!this.pending.has(id)) return;
        if (code === 'cancelled') this.stats.cancellations++; else this.stats.timeouts++;
        this.settle(id, new IpcError(code));
        if (sent && method.kind === 'rpc') this.sendControl(frame(this.generation, id, Kind.CANCEL));
      };
      const abort = () => cancel('cancelled');
      const timer = setTimeout(() => cancel('deadline-exceeded'), Math.max(0, deadline - performance.now()));
      options.signal?.addEventListener('abort', abort, { once: true });
      this.pending.set(id, {
        kind: method.kind, ...(method.kind === 'rpc' ? { codec: method.response } : {}),
        resolve, reject, cleanup: () => { clearTimeout(timer); options.signal?.removeEventListener('abort', abort); control?.release(); },
      });
      if (options.signal?.aborted) { cancel('cancelled'); return; }
      void this.output!.send(encoded, false, original => {
        if (!this.pending.has(id)) return undefined;
        const remaining = Math.ceil(deadline - performance.now());
        if (remaining <= 0) { cancel('deadline-exceeded'); return undefined; }
        const outgoing = IpcFrame.decode(original);
        sent = true;
        if (method.kind !== 'rpc' || outgoing.timeoutMs === remaining) return original;
        outgoing.timeoutMs = remaining;
        return IpcFrame.encode(outgoing).finish();
      }, id).catch(error => {
        const failure = ipcError(error);
        this.settle(id, failure);
        if (failure.code === 'transport' || failure.code === 'closed' || failure.code === 'navigated') this.fail(failure);
      });
    });
  }

  private settle(id: bigint, error?: IpcError, value?: unknown): void {
    const pending = this.pending.get(id);
    if (!pending) return;
    this.pending.delete(id);
    this.output?.remove(id);
    pending.cleanup();
    if (error) pending.reject(error); else pending.resolve(value);
  }
  private receive(bytes: Uint8Array, credit?: InputCredit): void {
    if (this.failure) { credit?.release(); return; }
    let transferred = false;
    try {
      const message = decodeFrame(bytes, this.limits);
      if (message.generation !== this.generation) { this.stats.staleReplies++; return; }
      if (message.kind === Kind.REQUEST || message.kind === Kind.NOTIFY) {
        if (message.id <= this.lastIncoming) throw new IpcError('invalid-frame');
        this.lastIncoming = message.id;
        try {
          credit ??= this.ledger.reserveInput(bytes.buffer.byteLength);
        } catch (error) { this.sendError(message.id, ipcError(error)); return; }
        transferred = this.dispatch(message, credit);
      }
      else if (message.kind === Kind.CANCEL) this.cancelIncoming(message.id);
      else this.complete(message);
    } catch (error) { this.fail(ipcError(error, 'invalid-frame')); }
    finally { if (!transferred) credit?.release(); }
  }
  private complete(message: IpcFrame): void {
    const pending = this.pending.get(message.id);
    if (!pending) { this.stats.staleReplies++; return; }
    if (message.kind === Kind.ERROR && message.body?.$case === 'error') {
      this.settle(message.id, fromWire(message.body.value)); return;
    }
    if (pending.kind === 'notification' && message.kind === Kind.ACCEPT) { this.settle(message.id); return; }
    if (pending.kind !== 'rpc' || message.kind !== Kind.RESULT || message.body?.$case !== 'payload') throw new IpcError('invalid-frame');
    try { this.settle(message.id, undefined, decodePayload(pending.codec!, message.body.value, this.limits)); }
    catch (error) { this.settle(message.id, ipcError(error, 'invalid-payload')); }
  }
  private dispatch(message: IpcFrame, credit: InputCredit): boolean {
    try {
      const method = this.methods.get(message.methodId);
      if (!method) throw new IpcError('unknown-method');
      if (method.receiver !== 'renderer' || (method.kind === 'rpc') !== (message.kind === Kind.REQUEST)) throw new IpcError('invalid-frame');
      const value = decodePayload(method.request, (message.body as { value: Uint8Array }).value, this.limits);
      if (method.kind === 'notification') return this.deliver(message, value, credit);
      this.runHandler(message, method, value, credit);
      return true;
    } catch (error) { this.sendError(message.id, ipcError(error, 'handler')); return false; }
  }
  private deliver(message: IpcFrame, value: unknown, credit: InputCredit): boolean {
    const callbacks = this.callbacks.get(message.methodId);
    if (this.callbackTasks + (callbacks?.size ?? 0) > this.limits.maxCallbackTasksPerDocument) throw new IpcError('overloaded');
    this.sendControl(frame(this.generation, message.id, Kind.ACCEPT), this.output!.reserveControl());
    if (this.failure || !callbacks?.size) return false;
    let remaining = callbacks.size;
    for (const callback of callbacks) {
      this.callbackTasks++;
      callback.tail = callback.tail.then(async () => {
        if (callback.active && !this.failure) await callback.invoke?.(value);
      }).catch(error => { this.stats.callbackErrors++; this.report(ipcError(error, 'handler')); })
        .finally(() => { this.callbackTasks--; if (--remaining === 0) credit.release(); });
    }
    return true;
  }
  private runHandler(message: IpcFrame, method: RpcDescriptor<any, any>, value: unknown, credit: InputCredit): void {
    const handler = this.handlers.get(method.id);
    if (!handler?.invoke) throw new IpcError('receiver-unavailable');
    if (this.handlerTasks >= this.limits.maxPendingCallsPerDirection) throw new IpcError('overloaded');
    const control = this.output!.reserveControl();
    this.handlerTasks++;
    const abort = new AbortController();
    const context = { signal: abort.signal, deadlineMs: performance.now() + message.timeoutMs };
    const timer = setTimeout(() => {
      if (!this.incoming.has(message.id)) return;
      this.cancelIncoming(message.id);
      this.sendError(message.id, new IpcError('deadline-exceeded'));
    }, message.timeoutMs);
    this.incoming.set(message.id, { abort, timer, control });
    void Promise.resolve().then(() => {
      if (abort.signal.aborted || this.failure) throw new IpcError('cancelled');
      if (this.handlers.get(method.id) !== handler || !handler.invoke) throw new IpcError('receiver-unavailable');
      return handler.invoke(value, context);
    }).then(result => {
      if (!this.incoming.has(message.id) || this.failure) return;
      const response = frame(this.generation, message.id, Kind.RESULT);
      const bytes = encodePayloadFrame(method.response, result, response, this.ledger);
      return this.output!.send(bytes, false, () => {
        if (!this.incoming.has(message.id) || this.failure) return undefined;
        if (performance.now() >= context.deadlineMs) {
          this.cancelIncoming(message.id);
          this.sendError(message.id, new IpcError('deadline-exceeded'));
          return undefined;
        }
        clearTimeout(timer);
        this.incoming.delete(message.id);
        return bytes;
      });
    }).catch(error => {
      const failure = ipcError(error, 'handler');
      if (failure.code === 'transport') this.fail(failure);
      else if (this.incoming.has(message.id) && !this.failure) this.sendError(message.id, failure, control);
    }).finally(() => {
      clearTimeout(timer);
      this.incoming.delete(message.id);
      this.handlerTasks--;
      credit.release();
      control.release();
    });
  }
  private cancelIncoming(id: bigint): void {
    const incoming = this.incoming.get(id);
    if (!incoming) return;
    this.incoming.delete(id);
    clearTimeout(incoming.timer);
    incoming.abort.abort();
    incoming.control.release();
  }
  private sendError(id: bigint, error: IpcError, credit?: ControlCredit): void {
    const response = frame(this.generation, id, Kind.ERROR);
    response.body = { $case: 'error', value: toWire(error) };
    this.sendControl(response, credit);
  }
  private sendControl(message: IpcFrame, credit?: ControlCredit): void {
    if (this.failure) return;
    void this.output!.send(IpcFrame.encode(message).finish(), true, undefined, undefined, credit).catch(error => this.fail(ipcError(error)));
  }
  private report(error: IpcError): void {
    const surface = (failure: IpcError) => {
      if (typeof globalThis.reportError === 'function') globalThis.reportError(failure);
      else queueMicrotask(() => { throw failure; });
    };
    if (this.options.onError) {
      try { this.options.onError(error); } catch { surface(new IpcError('handler', 'onError callback failed.')); }
    } else surface(error);
  }
  private fail(error: IpcError): void {
    if (this.failure) return;
    this.failure = error;
    for (const id of this.pending.keys()) this.settle(id, error);
    for (const id of this.incoming.keys()) this.cancelIncoming(id);
    for (const callbacks of this.callbacks.values()) for (const callback of callbacks) {
      callback.active = false;
      delete callback.invoke;
    }
    this.callbacks.clear();
    for (const handler of this.handlers.values()) delete handler.invoke;
    this.handlers.clear();
    this.callbackCount = 0;
    this.output?.close(error);
    this.transport.close();
    this.resolveClosed(error);
  }
}

/** Connect once to this document; no automatic retries or retargeting. */
export async function createConnection(schema: IpcSchema, transport: IpcTransport, options: ConnectionOptions = {}): Promise<Connection> {
  const connection = new Connection(schema, transport, options);
  await connection.start();
  return connection;
}
