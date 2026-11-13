// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { abortable } from './abortable.js';
import type { RouteResponseInfo, RouteResponsePolicy } from './types.js';

export async function inspectResponse(
  policy: RouteResponsePolicy,
  response: RouteResponseInfo,
  path: string,
  signal: AbortSignal,
  speculative: boolean,
): Promise<void> {
  signal.throwIfAborted();
  const pending = policy.inspect(response, { path, signal, speculative });
  if (pending) await abortable(pending, signal);
  signal.throwIfAborted();
}

export async function cancelResponse(response: Response, reason?: unknown): Promise<void> {
  if (!response.body || response.body.locked) return;
  // Aborting a native fetch can already have errored its body. Keep the original failure.
  await response.body.cancel(reason).catch(() => {});
}
