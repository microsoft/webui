// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { errorCode, IpcError, ipcError } from './errors.js';
import { defaultLimits, validateLimits, type IpcLimits } from './limits.js';
import { projectHello } from './hello.js';
import type { DocumentActivation, Hello, NativeControl, NativeIpcBootstrap, SessionInfo, Subscription } from './types.js';

/** Native-only adapter surface. Application DTOs never pass through this channel. */
export interface NativeChannel {
  postMessage(message: Readonly<Record<string, unknown>>): unknown;
  subscribe(listener: (message: unknown) => void): Subscription;
}

function isHex(value: unknown, length: number): value is string {
  if (typeof value !== 'string' || value.length !== length) return false;
  for (let i = 0; i < value.length; i++) {
    const code = value.charCodeAt(i);
    if (!(code >= 48 && code <= 57) && !(code >= 97 && code <= 102)) return false;
  }
  return true;
}
function generation(value: unknown, allowZero = false): value is string {
  if (typeof value !== 'string' || value.length === 0 || value.length > 20) return false;
  try { const number = BigInt(value); return number >= (allowZero ? 0n : 1n) && number <= 0xffffffffffffffffn && String(number) === value; }
  catch { return false; }
}
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new IpcError('invalid-frame');
  const serialized = JSON.stringify(value);
  if (serialized.length > defaultLimits.maxNativeControlBytes ||
      new TextEncoder().encode(serialized).byteLength > defaultLimits.maxNativeControlBytes) throw new IpcError('invalid-frame');
  return value as Record<string, unknown>;
}

interface BootstrapEpoch {
  readonly bootstrap: NativeIpcBootstrap;
  readonly generation: bigint;
  retire(): void;
}

function createBootstrapEpoch(channel: NativeChannel, retiredGeneration = 0n): BootstrapEpoch {
  let documentNonce = '';
  for (const byte of crypto.getRandomValues(new Uint8Array(16))) documentNonce += byte.toString(16).padStart(2, '0');
  let activation: Readonly<DocumentActivation> | undefined;
  let activateWait: (() => void) | undefined;
  let listener: ((control: NativeControl) => void) | undefined;
  let attempted = false;
  let closed = false;
  let admitted: { generation: string; token: string } | undefined;
  let lastGeneration = retiredGeneration;
  let pending: { resolve(value: SessionInfo): void; reject(error: IpcError): void; timer: ReturnType<typeof setTimeout> } | undefined;
  const callId = '1';
  const nativeListener = channel.subscribe(receive);

  function postDisconnect(value: { generation: string; token: string }): void {
    try {
      void Promise.resolve(channel.postMessage({ kind: 'disconnect', generation: value.generation, token: value.token }))
        .catch(() => { /* Native navigation may have already revoked the document. */ });
    } catch { /* Local revocation is already complete. */ }
  }
  function disconnectNative(): void {
    if (!admitted) return;
    const value = admitted;
    admitted = undefined;
    postDisconnect(value);
  }
  function matchesProof(message: Record<string, unknown>, proof: Readonly<DocumentActivation>): boolean {
    return message.kind === 'helloResult' && message.callId === callId &&
      message.navigation === proof.navigation && message.documentNonce === proof.documentNonce &&
      message.challenge === proof.challenge;
  }
  function retireLateReply(raw: unknown, proof: Readonly<DocumentActivation>): void {
    try {
      const message = record(raw);
      if (matchesProof(message, proof) && !message.error && generation(message.generation) && isHex(message.token, 32)) {
        postDisconnect({ generation: message.generation, token: message.token });
      }
    } catch { /* A malformed late result grants no session authority. */ }
  }
  function fail(error: IpcError): void {
    if (!pending) return;
    const current = pending;
    pending = undefined;
    activateWait = undefined;
    activation = undefined;
    clearTimeout(current.timer);
    closed = true;
    nativeListener.close();
    listener = undefined;
    disconnectNative();
    current.reject(error);
  }
  function receive(raw: unknown): void {
    if (closed) return;
    try {
      const message = record(raw);
      if (message.kind === 'ready' || message.kind === 'closed') {
        if (!generation(message.generation)) throw new IpcError('invalid-frame');
        if (BigInt(message.generation) <= retiredGeneration) return;
        if (message.kind === 'ready') listener?.({ kind: 'ready', generation: message.generation });
        else {
          const code = errorCode(message.code);
          listener?.({ kind: 'closed', generation: message.generation, code });
          if (!admitted || message.generation === admitted.generation) fail(new IpcError(code));
        }
        return;
      }
      if (!pending || !activation || !matchesProof(message, activation)) return;
      if (message.error) {
        const error = record(message.error);
        fail(new IpcError(errorCode(error.code)));
        return;
      }
      if (!generation(message.generation)) throw new IpcError('invalid-frame');
      if (!isHex(message.token, 32)) throw new IpcError('invalid-frame');
      admitted = { generation: message.generation, token: message.token };
      if (BigInt(message.generation) <= retiredGeneration) throw new IpcError('navigated');
      lastGeneration = BigInt(message.generation);
      const limits = record(message.limits) as unknown as IpcLimits;
      validateLimits(limits);
      const current = pending;
      pending = undefined;
      clearTimeout(current.timer);
      current.resolve({ generation: message.generation, token: message.token, limits: Object.freeze({ ...limits }) });
    } catch (error) { fail(ipcError(error, 'invalid-frame')); }
  }

  const bootstrap: NativeIpcBootstrap = {
    documentNonce,
    activate(proof): boolean {
      if (closed || !proof || proof.documentNonce !== documentNonce ||
          !generation(proof.navigation, true) || !isHex(proof.challenge, 32)) return false;
      if (activation) return activation.navigation === proof.navigation && activation.challenge === proof.challenge;
      activation = Object.freeze({ navigation: proof.navigation, documentNonce, challenge: proof.challenge });
      const resume = activateWait;
      activateWait = undefined;
      resume?.();
      return true;
    },
    hello(hello: Hello): Promise<SessionInfo> {
      if (attempted || closed) return Promise.reject(new IpcError('not-ready'));
      attempted = true;
      try {
        hello = projectHello(hello);
        if (hello.wireVersion !== 3 || !hello.contractName || hello.contractName.length > 1024 ||
            !Number.isSafeInteger(hello.contractMajor) || hello.contractMajor <= 0 ||
            hello.contractMajor > 0xffffffff || !isHex(hello.schemaHash, 64)) throw new IpcError('invalid-payload');
      } catch (error) {
        closed = true;
        nativeListener.close();
        listener = undefined;
        return Promise.reject(ipcError(error, 'invalid-payload'));
      }
      return new Promise((resolve, reject) => {
        pending = { resolve, reject, timer: setTimeout(() => {
          fail(new IpcError('deadline-exceeded', 'Native IPC admission timed out.'));
          closed = true;
          nativeListener.close();
          listener = undefined;
        }, defaultLimits.handshakeTimeoutMs) };
        const send = () => {
          if (!activation) return;
          const proof = activation;
          void Promise.resolve().then(() => {
            if (!pending || closed) return;
            const message = { ...hello, ...proof, kind: 'hello', callId };
            record(message);
            return channel.postMessage(message);
          }).then(reply => {
            if (reply === undefined) return;
            if (closed) retireLateReply(reply, proof); else receive(reply);
          })
            .catch(error => fail(ipcError(error)));
        };
        if (activation) send(); else activateWait = send;
      });
    },
    subscribeControl(callback): Subscription {
      if (closed) throw new IpcError('closed');
      if (listener) throw new IpcError('invalid-frame', 'A transport already owns this document.');
      listener = callback;
      return { close() { if (listener === callback) listener = undefined; } };
    },
    disconnect(value, token): void {
      if (closed) return;
      if (admitted) {
        if (value !== admitted.generation || token !== admitted.token) return;
      } else if (value !== '0' || token !== '') return;
      closed = true;
      activateWait = undefined;
      activation = undefined;
      fail(new IpcError('closed'));
      nativeListener.close();
      listener = undefined;
      disconnectNative();
    },
  };
  return {
    bootstrap: Object.freeze(bootstrap),
    get generation() { return lastGeneration; },
    retire(): void {
      const waiting = pending;
      const notify = listener;
      const value = admitted?.generation ?? '0';
      closed = true;
      pending = undefined;
      listener = undefined;
      activation = undefined;
      activateWait = undefined;
      if (waiting) clearTimeout(waiting.timer);
      nativeListener.close();
      disconnectNative();
      waiting?.reject(new IpcError('navigated'));
      notify?.({ kind: 'closed', generation: value, code: 'navigated' });
    },
  };
}

/** Normalize reply-capable WK/GTK and correlated WebView2 controls for one epoch. */
export function createNativeBootstrap(channel: NativeChannel): NativeIpcBootstrap {
  return createBootstrapEpoch(channel).bootstrap;
}

/** Install only from the main-frame native document-start bootstrap. */
export function installNativeBootstrap(
  target: object,
  channel: NativeChannel,
  subscribePageHide?: (listener: (persisted: boolean) => void) => void,
): NativeIpcBootstrap {
  if ('__webuiDesktopIpcV2' in target) throw new IpcError('invalid-frame', 'Conflicting IPC bootstrap.');
  if (!Object.isExtensible(target)) throw new IpcError('invalid-frame', 'IPC bootstrap target is not extensible.');
  let current = createBootstrapEpoch(channel);
  const bootstrap: NativeIpcBootstrap = Object.freeze({
    get documentNonce() { return current.bootstrap.documentNonce; },
    activate: (proof: DocumentActivation) => current.bootstrap.activate(proof),
    hello: (hello: Hello) => current.bootstrap.hello(hello),
    subscribeControl: (listener: (control: NativeControl) => void) => current.bootstrap.subscribeControl(listener),
    disconnect: (generation: string, token: string) => current.bootstrap.disconnect(generation, token),
  });
  Object.defineProperty(target, '__webuiDesktopIpcV2', { value: bootstrap, writable: false, configurable: false });
  subscribePageHide?.(persisted => {
    const previous = current;
    try { previous.retire(); }
    finally {
      // Prepare before BFCache freezes the realm; native restore probes can
      // precede pageshow. Only admission state changes, never old connections.
      if (persisted) current = createBootstrapEpoch(channel, previous.generation);
    }
  });
  return bootstrap;
}
