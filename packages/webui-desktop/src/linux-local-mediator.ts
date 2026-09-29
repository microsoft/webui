// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Only inject this in the named isolated world of the TOP frame. A WebKitGTK
// handler registered for this world is technically callable by children IF
// code is injected there. Do not change injection to AllFrames.
export {};

const marker = 'webui.linux.local.ipc.v1';
const maxControl = 4096;
const maxData = 4 * (24 * 1024 / 3) + 1024;

type BridgeRequest = {
  marker: string;
  direction: string;
  lane: 'control' | 'data';
  id: number;
  payload: Record<string, unknown>;
};
type IsolatedWindow = Window & {
  webkit?: { messageHandlers?: {
    webuiDesktopIpc?: { postMessage(message: unknown): Promise<unknown> };
    webuiDesktopIpcData?: { postMessage(message: unknown): Promise<unknown> };
  } };
};

function bounded(payload: Record<string, unknown>, lane: 'control' | 'data'): boolean {
  const keys = Object.keys(payload);
  if (keys.length === 0 || keys.length > 10) return false;
  let units = 32;
  for (const key of keys) {
    if (key.length > 32) return false;
    const value = payload[key];
    if (typeof value !== 'string' && typeof value !== 'number') return false;
    if (typeof value === 'number' && !Number.isSafeInteger(value)) return false;
    const size = typeof value === 'string' ? value.length : 20;
    if (size > (lane === 'control' ? maxControl : maxData)) return false;
    units += key.length + size + 8;
    if (units > (lane === 'control' ? maxControl : maxData)) return false;
  }
  return true;
}

if (window === window.top) {
  const isolated = window as IsolatedWindow;
  const control = isolated.webkit?.messageHandlers?.webuiDesktopIpc;
  const data = isolated.webkit?.messageHandlers?.webuiDesktopIpcData;
  if (control && data) {
    const pending = new Set<number>();
    window.addEventListener('message', event => {
      // Read source and origin BEFORE looking at event.data. Cross-origin
      // postMessage carries the child's source; it cannot forge window here.
      if (event.source !== window || event.origin !== location.origin) return;
      const request = event.data as BridgeRequest | null;
      if (!request || typeof request !== 'object' || Array.isArray(request) ||
          request.marker !== marker || request.direction !== 'page-to-native' ||
          (request.lane !== 'control' && request.lane !== 'data') ||
          !Number.isSafeInteger(request.id) || request.id < 1 ||
          pending.has(request.id) || pending.size >= 4 ||
          !request.payload || typeof request.payload !== 'object' ||
          Array.isArray(request.payload) ||
          !bounded(request.payload, request.lane)) return;
      // Every top-level field is primitive and bounded before passing through
      // the isolated handler. Native repeats validation before Rust copying.
      pending.add(request.id);
      const handler = request.lane === 'control' ? control : data;
      Promise.resolve(handler.postMessage(request.payload)).then(
        payload => window.postMessage({
          marker, direction: 'native-to-page', kind: 'reply',
          lane: request.lane, id: request.id, ok: true, payload,
        }, location.origin),
        () => window.postMessage({
          marker, direction: 'native-to-page', kind: 'reply',
          lane: request.lane, id: request.id, ok: false,
        }, location.origin),
      ).finally(() => pending.delete(request.id));
    });
  }
}
