// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Whole-request defaults, including an unread or deferred state tail. */
export const DEFAULT_TIMEOUT_MS = 10_000;

export function validateTimeout(timeoutMs: number): void {
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
    throw new RangeError('[WebUI] timeoutMs must be positive and finite.');
  }
}

/** Request lifetime shared by route and region consumers; neither owns shared resources. */
export class ResourceRequest {
  readonly controller = new AbortController();
  readonly signal: AbortSignal;
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor(private readonly externalSignal?: AbortSignal, timeoutMs = DEFAULT_TIMEOUT_MS) {
    validateTimeout(timeoutMs);
    externalSignal?.throwIfAborted();
    this.signal = this.controller.signal;
    this.timer = setTimeout(() => {
      this.controller.abort(new DOMException('Partial response timed out', 'TimeoutError'));
    }, timeoutMs);
    this.signal.addEventListener('abort', this.dispose, { once: true });
    externalSignal?.addEventListener('abort', this.abort, { once: true });
  }

  private readonly abort = (): void => this.controller.abort(this.externalSignal?.reason);

  readonly dispose = (): void => {
    clearTimeout(this.timer);
    this.timer = undefined;
    this.signal.removeEventListener('abort', this.dispose);
    this.externalSignal?.removeEventListener('abort', this.abort);
  };
}

/** Resource availability is build-scoped; state freshness is not inferred from it. */
export function resourceHeaders(accept: string): Record<string, string> {
  const runtime = window.__webui;
  const headers: Record<string, string> = { Accept: accept };
  if (runtime?.inventory) headers['X-WebUI-Inventory'] = runtime.inventory;
  if (runtime?.styleInventory) headers['X-WebUI-Style-Inventory'] = runtime.styleInventory;
  if (runtime?.buildId) headers['X-WebUI-Build'] = runtime.buildId;
  return headers;
}
