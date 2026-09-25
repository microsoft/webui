// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(`Windows resource transport: ${message}`);
}

async function aborted(result: Promise<unknown>): Promise<void> {
  try {
    await result;
  } catch (error) {
    assert(error instanceof DOMException && error.name === 'AbortError', `expected AbortError, got ${String(error)}`);
    return;
  }
  throw new Error('Windows resource transport: aborted fetch resolved');
}

async function workerResource(shared: boolean): Promise<void> {
  const resource = new URL('/fixture-runtime.js', location.href).href;
  const operation = `fetch(${JSON.stringify(resource)}).then(async response => {
    if (!response.ok || response.url !== ${JSON.stringify(resource)}) throw new Error('worker response');
    return response.text();
  })`;
  const script = shared
    ? `onconnect = event => { const port = event.ports[0]; ${operation}.then(
        text => port.postMessage({ text }), error => port.postMessage({ error: String(error) })); };`
    : `${operation}.then(
        text => postMessage({ text }), error => postMessage({ error: String(error) }));`;
  const url = URL.createObjectURL(new Blob([script], { type: 'text/javascript' }));
  const worker = shared ? new SharedWorker(url) : new Worker(url);
  const channel = worker instanceof SharedWorker ? worker.port : worker;
  try {
    const result = await new Promise<{ text?: string; error?: string }>((resolve, reject) => {
      channel.onmessage = event => resolve(event.data);
      channel.onmessageerror = () => reject(new Error('worker message decoding failed'));
      worker.onerror = event => reject(new Error(event.message));
      if (channel instanceof MessagePort) channel.start();
    });
    assert(!result.error && result.text === 'export const servedByRuntime = true;', 'worker native interception');
  } finally {
    if (worker instanceof SharedWorker) worker.port.close();
    else worker.terminate();
    URL.revokeObjectURL(url);
  }
}

export async function checkWindowsResources(): Promise<void> {
  if (!navigator.userAgent.includes('Windows')) return;
  assert(Function.prototype.toString.call(window.fetch).includes('[native code]'), 'fetch was replaced');
  assert(!('__webuiDesktopFetchBridge' in window), 'legacy resource bridge installed');
  const url = new URL('/fixture-runtime.js', location.href).href;
  const response = await fetch(new Request(url, { cache: 'no-store' }));
  assert(response.ok && response.url === url && response.type === 'basic', 'native Response metadata');
  assert(response.headers.get('content-type')?.includes('javascript'), 'content type');
  const clone = response.clone();
  assert(await response.text() === 'export const servedByRuntime = true;', 'runtime-only resource');
  assert(response.bodyUsed && await clone.text() === 'export const servedByRuntime = true;', 'body/clone semantics');
  const head = await fetch(url, { method: 'HEAD' });
  assert(head.ok && (await head.arrayBuffer()).byteLength === 0, 'HEAD body must be empty');

  const bytes = Uint8Array.from({ length: 16384 }, (_, index) => index % 256);
  const echo = await fetch(new Request(new URL('/fixture-resource-echo', location.href), {
    method: 'POST', body: bytes,
  }));
  const echoed = new Uint8Array(await echo.arrayBuffer());
  assert(echo.ok && echoed.length === bytes.length && echoed.every((byte, index) => byte === bytes[index]),
    'binary request/response stream');

  await aborted(fetch(url, { signal: AbortSignal.abort() }));
  const controller = new AbortController();
  const pending = fetch(url, { signal: controller.signal });
  controller.abort();
  await aborted(pending);
  const bodyController = new AbortController();
  const body = await fetch(url, { signal: bodyController.signal });
  bodyController.abort();
  await aborted(body.arrayBuffer());
  assert((await fetch(url)).ok, 'native fetch must recover after cancellation');
  await workerResource(false);
  await workerResource(true);
  console.info('NATIVE_WINDOWS_RESOURCES_OK');
}
