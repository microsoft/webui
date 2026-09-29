// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { installNativeBootstrap, type NativeChannel } from './bootstrap.js';
import type { NativeDataLane } from './types.js';
import { IpcError } from './errors.js';

interface WebViewChannel {
  postMessage(message: unknown): void;
  addEventListener(name: 'message', listener: (event: MessageEvent) => void): void;
  removeEventListener(name: 'message', listener: (event: MessageEvent) => void): void;
}
interface NativeWindow extends Window {
  webkit?: { messageHandlers?: {
    webuiDesktopIpc?: { postMessage(message: unknown): unknown };
    webuiDesktopIpcData?: { postMessage(message: unknown): unknown };
  } };
  chrome?: { webview?: WebViewChannel };
}

declare const __WEBUI_NATIVE_CARRIER__: boolean;

/** Native-injected bootstrap shared by bundled and opt-in local-server documents. */
export function installNativeEntry(target: NativeWindow): void {
  // Native adapters also restrict injection to main frames.
  if (target !== target.top) return;
  const wk = target.webkit?.messageHandlers?.webuiDesktopIpc;
  const wkData = (typeof __WEBUI_NATIVE_CARRIER__ !== 'undefined' && __WEBUI_NATIVE_CARRIER__)
    ? target.webkit?.messageHandlers?.webuiDesktopIpcData : undefined;
  const webview = target.chrome?.webview;
  if (!wk && !webview) return;
  let receiver: ((value: unknown) => void) | undefined;
  let dataReceiver: ((value: unknown) => void) | undefined;
  const channel: NativeChannel = {
    postMessage: message => wk ? wk.postMessage(message) : webview!.postMessage(message),
    subscribe(listener) {
      receiver = listener;
      const onWebViewMessage = (event: MessageEvent) => {
        const message = event.data;
        if ((typeof __WEBUI_NATIVE_CARRIER__ !== 'undefined' && __WEBUI_NATIVE_CARRIER__) &&
            message && typeof message === 'object' && message.kind === 'ipcDataResult') dataReceiver?.(message);
        else listener(message);
      };
      let active = true;
      if (webview) webview.addEventListener('message', onWebViewMessage);
      return { close() {
        if (!active) return;
        active = false;
        if (receiver === listener) receiver = undefined;
        if (webview) webview.removeEventListener('message', onWebViewMessage);
      } };
    },
  };
  const dataLane: NativeDataLane | undefined =
    (typeof __WEBUI_NATIVE_CARRIER__ !== 'undefined' && __WEBUI_NATIVE_CARRIER__) && (wkData || webview) ? {
    postMessage: message => wkData ? wkData.postMessage(message) : webview!.postMessage(message),
    subscribe(listener) {
      if (dataReceiver) throw new IpcError('invalid-frame', 'A native data cursor already owns this document.');
      dataReceiver = listener;
      return { close() { if (dataReceiver === listener) dataReceiver = undefined; } };
    },
  } : undefined;
  if ('__webuiDesktopIpcReceiveV2' in target) throw new IpcError('invalid-frame', 'Conflicting native IPC control receiver.');
  // All backends use this hook for nonce-bound retirement. Other WebView2
  // controls use its message event; WK/GTK use the hook for those as well.
  Object.defineProperty(target, '__webuiDesktopIpcReceiveV2', {
    value: Object.freeze((message: unknown) => receiver?.(message)),
    writable: false,
    configurable: false,
  });
  if ((typeof __WEBUI_NATIVE_CARRIER__ !== 'undefined' && __WEBUI_NATIVE_CARRIER__) && wkData) {
    if ('__webuiDesktopIpcDataReceiveV1' in target) throw new IpcError('invalid-frame', 'Conflicting native IPC data receiver.');
    Object.defineProperty(target, '__webuiDesktopIpcDataReceiveV1', {
      value: Object.freeze((message: unknown) => dataReceiver?.(message)),
      writable: false,
      configurable: false,
    });
  }
  installNativeBootstrap(target, channel, listener => {
    target.addEventListener('pagehide', event => {
      if (event.isTrusted) listener(event.persisted);
    });
  }, dataLane);
}
