// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Page telemetry shared by the startup owners and the on-demand dialogs, the
 * way a real new tab page shares one logging module. Events are batched and
 * flushed at idle into an in-memory sink; the benchmark sends nothing.
 */

/** One queued telemetry event. */
export interface TelemetryEvent {
  name: string;
  time: number;
  data: Readonly<Record<string, unknown>>;
}

const BATCH = 16;
const RETAINED = 200;
const pending: TelemetryEvent[] = [];
let scheduled = false;

/** Queue an event; batches flush when the browser is idle. */
export function track(name: string, data: Readonly<Record<string, unknown>> = {}): void {
  pending.push({ name, time: Date.now(), data });
  if (scheduled) return;
  scheduled = true;
  const run = (): void => {
    scheduled = false;
    flush();
  };
  if (typeof requestIdleCallback === 'function') requestIdleCallback(run, { timeout: 2000 });
  else setTimeout(run, 200);
}

function flush(): void {
  const host = globalThis as { __ntpTelemetry?: TelemetryEvent[] };
  const sink = (host.__ntpTelemetry ??= []);
  while (pending.length > 0) sink.push(...pending.splice(0, BATCH));
  if (sink.length > RETAINED) sink.splice(0, sink.length - RETAINED);
}
