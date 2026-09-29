// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Linux-only main-world entry. This realm never receives a WebKit script
// handler: a TOP_FRAME-only isolated-world script owns the native calls.
import { installNativeBootstrap, type NativeChannel } from './bootstrap.js';
import type { NativeDataLane } from './types.js';

const marker = 'webui.linux.local.ipc.v1';
const maxPending = 4;
const maxCorrelation = Number.MAX_SAFE_INTEGER;
type Lane = 'control' | 'data';
type Pending = {
  lane: Lane;
  resolve(value: unknown): void;
  reject(error: Error): void;
  timer: ReturnType<typeof setTimeout>;
};

if (window === window.top) {
  let nextId = 0;
  const pending = new Map<number, Pending>();
  let controlReceiver: ((value: unknown) => void) | undefined;
  let dataReceiver: ((value: unknown) => void) | undefined;

  function post(lane: Lane, payload: Readonly<Record<string, unknown>>): Promise<unknown> {
    if (pending.size >= maxPending || nextId === maxCorrelation) {
      return Promise.reject(new Error('Native IPC bridge overloaded'));
    }
    const id = ++nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error('Native IPC bridge deadline exceeded'));
      }, 5000);
      pending.set(id, { lane, resolve, reject, timer });
      window.postMessage({ marker, direction: 'page-to-native', lane, id, payload }, location.origin);
    });
  }

  window.addEventListener('message', event => {
    if (event.source !== window || event.origin !== location.origin) return;
    const value = event.data;
    if (!value || typeof value !== 'object' || Array.isArray(value) ||
        value.marker !== marker || value.direction !== 'native-to-page' ||
        (value.lane !== 'control' && value.lane !== 'data') ||
        value.kind !== 'reply' || !Number.isSafeInteger(value.id) || value.id < 1) return;
    const current = pending.get(value.id);
    if (!current || current.lane !== value.lane) return;
    pending.delete(value.id);
    clearTimeout(current.timer);
    if (value.ok === true) current.resolve(value.payload);
    else current.reject(new Error('Native IPC bridge rejected message'));
  });

  const channel: NativeChannel = {
    postMessage: payload => post('control', payload),
    subscribe(listener) {
      if (controlReceiver) throw new Error('Duplicate native control receiver');
      controlReceiver = listener;
      return { close() { if (controlReceiver === listener) controlReceiver = undefined; } };
    },
  };
  const dataLane: NativeDataLane = {
    postMessage: payload => post('data', payload),
    subscribe(listener) {
      if (dataReceiver) throw new Error('Duplicate native data receiver');
      dataReceiver = listener;
      return { close() { if (dataReceiver === listener) dataReceiver = undefined; } };
    },
  };
  Object.defineProperty(window, '__webuiDesktopIpcReceiveV2', {
    value: Object.freeze((value: unknown) => controlReceiver?.(value)),
    configurable: false, writable: false,
  });
  Object.defineProperty(window, '__webuiDesktopIpcDataReceiveV1', {
    value: Object.freeze((value: unknown) => dataReceiver?.(value)),
    configurable: false, writable: false,
  });
  installNativeBootstrap(window, channel, listener => {
    window.addEventListener('pagehide', event => {
      if (event.isTrusted) listener(event.persisted);
    });
  }, dataLane);
}
