// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Exercise Fetch-visible MIME metadata and a genuinely lazy first connection. */
export async function checkAdmissionDelivery(): Promise<void> {
  const runtime = await fetch('/_webui/ipc/runtime.js');
  if (!runtime.ok || !runtime.headers.get('Content-Type')?.startsWith('text/javascript')) {
    throw new Error(`Native runtime MIME missing: ${runtime.status} ${runtime.headers.get('Content-Type')}`);
  }
  await runtime.arrayBuffer();
  const rejected = await fetch('/_webui/ipc/outbound');
  if (rejected.ok || rejected.headers.get('Content-Type') !== 'application/x-protobuf' ||
      rejected.headers.get('Cache-Control') !== 'no-store') {
    throw new Error(`Native IPC error MIME/cache headers missing: ${rejected.status} ${rejected.headers.get('Content-Type')}`);
  }
  await rejected.arrayBuffer();
  // Deliberately exceed the default handshake budget before starting hello.
  // This is a regression workload, not a readiness wait.
  await new Promise(resolve => setTimeout(resolve, 5500));
}
