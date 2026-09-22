// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { installNativeBootstrap, type NativeChannel } from './bootstrap.js';
import { IpcError } from './errors.js';

interface WebViewChannel {
  postMessage(message: unknown): void;
  addEventListener(name: 'message', listener: (event: MessageEvent) => void): void;
  removeEventListener(name: 'message', listener: (event: MessageEvent) => void): void;
}
interface NativeWindow extends Window {
  webkit?: { messageHandlers?: { webuiDesktopIpc?: { postMessage(message: unknown): unknown } } };
  chrome?: { webview?: WebViewChannel };
}

const target = window as NativeWindow;
// Native adapters also restrict injection to main frames. Keep a defensive check.
if (target === target.top) {
  const wk = target.webkit?.messageHandlers?.webuiDesktopIpc;
  const webview = target.chrome?.webview;
  if (wk || webview) {
    let receiver: ((value: unknown) => void) | undefined;
    const channel: NativeChannel = {
      postMessage: message => wk ? wk.postMessage(message) : webview!.postMessage(message),
      subscribe(listener) {
        receiver = listener;
        const onMessage = (event: MessageEvent) => listener(event.data);
        let active = true;
        if (webview) webview.addEventListener('message', onMessage);
        return { close() {
          if (!active) return;
          active = false;
          if (receiver === listener) receiver = undefined;
          if (webview) webview.removeEventListener('message', onMessage);
        } };
      },
    };
    if ('__webuiDesktopIpcReceiveV2' in target) throw new IpcError('invalid-frame', 'Conflicting native IPC control receiver.');
    // All backends use this hook for nonce-bound retirement. Other WebView2
    // controls use its message event; WK/GTK use the hook for those as well.
    Object.defineProperty(target, '__webuiDesktopIpcReceiveV2', {
      value: Object.freeze((message: unknown) => receiver?.(message)),
      writable: false,
      configurable: false,
    });
    installNativeBootstrap(target, channel, listener => {
      target.addEventListener('pagehide', event => {
        if (event.isTrusted) listener(event.persisted);
      });
    });
  }
}
