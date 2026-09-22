// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { IpcError, ipcError } from './errors.js';
import { decodeFrame, fromWire, Kind } from './framing.js';
import { WireError } from './generated/webui_desktop.js';
import { validateLimits } from './limits.js';
import { ByteLedger, hasLedger, type InputCredit } from './budget.js';
import { projectHello } from './hello.js';
import type { BinaryReceiver, Hello, IpcTransport, NativeIpcBootstrap, SessionInfo, Subscription } from './types.js';

const frameLedgers = new WeakMap<NativeIpcBootstrap, ByteLedger>();

export interface DesktopTransportOptions {
  /** Injection seams for non-GUI contract tests and custom embedders. */
  bootstrap?: NativeIpcBootstrap;
  fetch?: typeof globalThis.fetch;
}

/** Native admission plus authenticated, binary-only custom-protocol requests. */
export function createDesktopTransport(options: DesktopTransportOptions = {}): IpcTransport {
  const bootstrap = options.bootstrap ?? globalThis.window?.__webuiDesktopIpcV2;
  const documentNonce = bootstrap?.documentNonce;
  const fetcher = options.fetch ?? globalThis.fetch;
  let session: SessionInfo | undefined;
  let receiver: BinaryReceiver | undefined;
  let listener: Subscription | undefined;
  let dirty = false;
  let draining = false;
  let started = false;
  let failure: IpcError | undefined;
  let postActive = false;
  let rejectStart: ((error: IpcError) => void) | undefined;
  let ledger: ByteLedger | undefined;
  const abort = new AbortController();

  function close(error = new IpcError('closed')): void {
    if (failure) return;
    failure = error;
    rejectStart?.(error);
    rejectStart = undefined;
    abort.abort();
    listener?.close();
    listener = undefined;
    // Zero only cancels the local, not-yet-admitted handshake. It is never posted natively.
    try {
      if (bootstrap?.documentNonce === documentNonce) bootstrap?.disconnect(session?.generation ?? '0', session?.token ?? '');
    } catch { /* Local shutdown remains terminal. */ }
    session = undefined;
    receiver?.closed(error);
    receiver = undefined;
  }

  async function readBody(response: Response, maximum: number, input: boolean): Promise<{ bytes: Uint8Array; credit?: InputCredit }> {
    const declared = response.headers.get('Content-Length');
    if (declared && (!Number.isSafeInteger(Number(declared)) || Number(declared) > maximum || Number(declared) < 0)) {
      await response.body?.cancel();
      throw new IpcError('payload-too-large');
    }
    if (!response.body) return { bytes: new Uint8Array() };
    const reader = response.body.getReader();
    // One serialized GET may own a small control buffer independently of data
    // credits. Larger buffers acquire payload credit before allocation/growth.
    let result: Uint8Array;
    let credit: InputCredit | undefined;
    let offset = 0;
    try {
      const controlCapacity = ledger!.limits.maxErrorTextBytesTotal + 128;
      const capacity = Math.min(maximum, controlCapacity, declared ? Math.max(1, Number(declared)) : controlCapacity);
      result = new Uint8Array(capacity);
      while (true) {
        const chunk = await reader.read();
        if (chunk.done) break;
        if (offset + chunk.value.byteLength > maximum) throw new IpcError('payload-too-large');
        if (offset + chunk.value.byteLength > result.byteLength) {
          const capacity = Math.min(maximum, Math.max(result.byteLength * 2, offset + chunk.value.byteLength));
          const releaseCopy = credit ? ledger!.reserve(result.byteLength) : () => {};
          try {
            if (input && (credit || capacity > controlCapacity)) {
              if (credit) credit.grow(capacity - result.byteLength);
              else credit = ledger!.reserveInput(capacity);
            }
            const grown = new Uint8Array(capacity);
            grown.set(result.subarray(0, offset));
            result = grown;
          } finally { releaseCopy(); }
        }
        result.set(chunk.value, offset);
        offset += chunk.value.byteLength;
      }
      if (input) {
        const message = decodeFrame(result.subarray(0, offset), ledger!.limits);
        if (!credit && (message.kind === Kind.REQUEST || message.kind === Kind.NOTIFY || message.kind === Kind.RESULT)) {
          // Small application frames cannot use the control reserve as an
          // uncharged payload path, even when their encoded payload is empty.
          credit = ledger!.reserveInput(result.byteLength);
        }
      }
    } catch (error) {
      credit?.release();
      await reader.cancel().catch(() => {});
      throw error;
    } finally { reader.releaseLock(); }
    // Credit tracks capacity, not view length, through actual handler completion.
    return { bytes: result.subarray(0, offset), ...(credit ? { credit } : {}) };
  }

  async function request(path: string, body?: Uint8Array): Promise<{ bytes: Uint8Array; credit?: InputCredit } | undefined> {
    if (failure) throw failure;
    const current = session;
    if (!current) throw new IpcError('not-ready');
    const headers: Record<string, string> = { 'X-WebUI-Ipc-Session': current.token };
    if (body) headers['Content-Type'] = 'application/x-protobuf';
    const init: RequestInit = { method: body ? 'POST' : 'GET', headers, cache: 'no-store', credentials: 'omit', redirect: 'error', signal: abort.signal };
    if (body) init.body = body as Uint8Array<ArrayBuffer>;
    // WebKit can cancel pulls before pagehide; keep them owned by our abort
    // signal. Payload POSTs must not enter the browser's 64 KiB keepalive quota.
    else init.keepalive = true;
    const response = await fetcher(path, init);
    if (failure) throw failure;
    if (response.status === 204) return undefined;
    if (!response.ok && ![400, 401, 409, 413, 429, 503].includes(response.status)) {
      await response.body?.cancel();
      throw new IpcError('transport');
    }
    if (response.ok && (body || response.status !== 200)) {
      await response.body?.cancel();
      throw new IpcError('transport');
    }
    if (response.headers.get('Content-Type')?.split(';')[0] !== 'application/x-protobuf') throw new IpcError('transport');
    const result = await readBody(response, response.ok ? current.limits.maxFrameBytes : current.limits.maxErrorTextBytesTotal + 128, response.ok);
    if (failure) {
      result.credit?.release();
      throw failure;
    }
    if (!response.ok) {
      try { throw fromWire(WireError.decode(result.bytes)); }
      catch (error) { throw ipcError(error); }
    }
    return result;
  }

  async function drain(): Promise<void> {
    if (draining || !session || failure) return;
    draining = true;
    try {
      do {
        dirty = false;
        while (!failure) {
          const result = await request('/_webui/ipc/outbound');
          if (!result) break;
          if (receiver && hasLedger(receiver) && result.credit) {
            await receiver.receiveReserved(result.bytes, result.credit);
          } else {
            try { await receiver?.receive(result.bytes); }
            finally { result.credit?.release(); }
          }
        }
      } while (dirty && !failure);
    } catch (error) { close(failure ?? ipcError(error)); }
    finally {
      draining = false;
      // A synchronous control callback during final completion must not be lost.
      if (dirty && !failure) void drain();
    }
  }

  return {
    async start(hello: Hello, target: BinaryReceiver): Promise<SessionInfo> {
      if (started || failure) throw failure ?? new IpcError('not-ready');
      started = true;
      receiver = target;
      try {
        if (!bootstrap) throw new IpcError('not-ready', 'Native IPC bootstrap is absent.');
        if (bootstrap.documentNonce !== documentNonce) throw new IpcError('navigated');
        const admissionHello = projectHello(hello);
        listener = bootstrap.subscribeControl(control => {
          if (failure) return;
          if (session && control.generation !== session.generation) return;
          if (control.kind === 'closed') { close(new IpcError(control.code)); return; }
          dirty = true;
          if (session) void drain();
        });
        const stopped = new Promise<never>((_resolve, reject) => { rejectStart = reject; });
        const admitted = await Promise.race([bootstrap.hello(admissionHello).then(admitted => {
          if (failure && bootstrap.documentNonce === documentNonce) bootstrap.disconnect(admitted.generation, admitted.token);
          return admitted;
        }), stopped]);
        rejectStart = undefined;
        if (failure) {
          if (bootstrap.documentNonce === documentNonce) bootstrap.disconnect(admitted.generation, admitted.token);
          throw failure;
        }
        session = admitted;
        validateLimits(admitted.limits);
        ledger = hasLedger(target) ? target.byteLedger : new ByteLedger(admitted.limits);
        const previousLedger = frameLedgers.get(bootstrap);
        if (previousLedger) ledger.shareWith(previousLedger);
        frameLedgers.set(bootstrap, ledger);
        ledger.limits = Object.freeze({ ...admitted.limits });
        if (dirty) void drain();
        return admitted;
      } catch (error) { close(ipcError(error)); throw failure; }
    },
    async send(bytes): Promise<void> {
      if (failure) throw failure;
      if (!session) throw new IpcError('not-ready');
      if (bytes.byteLength > session.limits.maxFrameBytes) throw new IpcError('payload-too-large');
      // The connection serializes ingress. Reject custom concurrent producers.
      if (postActive) throw new IpcError('overloaded');
      postActive = true;
      try { await request('/_webui/ipc', bytes); }
      catch (error) {
        // Trusted retirement may synchronously abort this fetch. Preserve the
        // terminal reason already recorded, not the browser's generic abort.
        const ipc = failure ?? ipcError(error);
        if (ipc.code !== 'overloaded' && ipc.code !== 'payload-too-large' && ipc.code !== 'invalid-payload') close(ipc);
        throw ipc;
      } finally { postActive = false; }
    },
    close: () => close(),
  };
}
