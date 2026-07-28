// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * In-page benchmark driver.
 *
 * `runScenario` runs entirely inside the browser via `page.evaluate`. Playwright
 * serialises only the passed function's own source, so this driver is a single
 * self-contained function with every helper nested inside it — no module-scope
 * references, no imports, no closed-over Node state. Doing the whole drive loop
 * in one evaluate keeps the measured "total scenario elapsed" free of
 * round-trip overhead and lets heap be sampled *while boundaries commit*.
 *
 * The only optional browser APIs are `performance.memory` (peak-heap sampling,
 * needs `--enable-precise-memory-info`) and `PerformanceObserver('longtask')`.
 * Both are tolerated when unsupported (reported as `null`); nothing else is
 * silently swallowed.
 */

/** Config handed to the driver. Boundary/terminal/body HTML is generated in
 *  Node (see `scenarios.ts`) and passed through as opaque strings. */
export interface DriveConfig {
  readonly mode: 'streaming' | 'ordinary';
  /** Streaming only: one HTML fragment per boundary, in order. */
  readonly boundaries?: readonly string[];
  /** Streaming only: the terminal envelope fragment. */
  readonly terminal?: string;
  /** Streaming only: when the `bench-island` class is registered. */
  readonly timing?: 'eager' | 'race';
  /** Correctness-only: capture the exact boundary state handed to each real
   *  WebUIElement activation. Disabled for measured runs. */
  readonly captureBoundaryState?: boolean;
  /** Ordinary only: SSR roots inserted after the baseline heap sample so
   *  peak-heap is measured on the same empty->populated transition as the
   *  streaming arms. The base document already contains `#webui-data`. */
  readonly bodyHtml?: string;
}

/** Metrics returned from a single browser run. */
export interface RunMetrics {
  /** Sum of real component mount/hydration CPU (ms) from the instrumented class. */
  cpuMs: number;
  /** Total scenario elapsed browser time (ms). For streaming this spans the
   *  whole append+commit pipeline (boundary parse + coordinator drains). For the
   *  ordinary control it is the synchronous hydration burst only (class define +
   *  upgrade of all roots); the one-time body parse is excluded so it compares
   *  like-for-like with per-boundary commit time. */
  elapsedMs: number;
  /** Peak `usedJSHeapSize` delta (bytes), or null when unsupported. Combines
   *  in-hook samples taken inside the instrumented hydration path (truthful
   *  "while committing" peak) with the driver's per-boundary samples. */
  peakHeapDeltaBytes: number | null;
  /** Longest `longtask` entry duration (ms), or null when unsupported. */
  maxLongTaskMs: number | null;
  /** Live `bench-island` roots after completion. */
  liveRootCount: number;
  /** Roots that reported a successful hydration exactly once (must equal the
   *  live root count / TOTAL_ROOTS): incremented by the instrumented subclass
   *  only after the real super hydration hook returns. */
  hydratedCount: number;
  /** Roots proven reactive by the post-measurement `setState` probe: their
   *  bound `<span>` text changed to the shared sentinel (must equal TOTAL_ROOTS). */
  verifiedReactiveCount: number;
  /** Residual boundary `<script>` scaffolding (must be 0). */
  scaffoldScripts: number;
  /** Residual `<webui-hydrate>` sentinels (must be 0). */
  scaffoldSentinels: number;
  /** Residual `wb:` / `/wb:` boundary comment markers (must be 0). */
  scaffoldComments: number;
  /** Residual `[data-ws]` streamed-host markers (must be 0). */
  scaffoldDataWs: number;
  /** Whether `window.__webui.state` is populated (must be false for streaming). */
  streamedStatePopulated: boolean;
  /** Correctness-only labels observed by the real deferred activation hook. */
  receivedBoundaryLabels?: string[];
}

/**
 * Drive one scenario and collect its metrics.
 *
 * Streaming: append boundaries one at a time, spinning the coordinator's
 * microtask pump until each boundary's `<webui-hydrate>` scaffolding is removed
 * (a deterministic "committed + cleaned" signal) before the next, then append
 * the terminal envelope and wait until every root is activated and all
 * scaffolding is gone. This DOM-based synchronisation avoids depending on the
 * opt-in `webui:boundary-hydrated` / completion diagnostics, whose firing is
 * coupled to evolving coordinator internals. For the `race` timing the class is
 * defined only after the first boundary commits, exercising the O(unique tag)
 * waiter path.
 *
 * Ordinary: defining the class synchronously upgrades every already-connected
 * SSR root, so `connectedCallback` (real hydration) runs inline and is timed by
 * the instrumented subclass.
 */
export async function runScenario(config: DriveConfig): Promise<RunMetrics> {
  const win = window as unknown as {
    __benchHydrationCpu?: number;
    __benchPeakHeap?: number;
    __benchHydratedCount?: number;
    __defineBenchIsland: () => void;
    __webui?: { state?: unknown };
  };

  function sampleHeap(): number | null {
    const mem = (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory;
    return mem ? mem.usedJSHeapSize : null;
  }

  function installLongTaskObserver(sink: number[]): PerformanceObserver | null {
    try {
      const observer = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) sink.push(entry.duration);
      });
      observer.observe({ type: 'longtask', buffered: true });
      return observer;
    } catch {
      // `longtask` is an optional entry type; treat absence as "no data".
      return null;
    }
  }

  function countBoundaryComments(): number {
    let count = 0;
    const walker = document.createNodeIterator(document.body, NodeFilter.SHOW_COMMENT);
    let node: Node | null = walker.nextNode();
    while (node) {
      const data = (node as Comment).data || '';
      if (data.indexOf('wb:') === 0 || data.indexOf('/wb:') === 0) count++;
      node = walker.nextNode();
    }
    return count;
  }

  win.__benchHydrationCpu = 0;
  win.__benchHydratedCount = 0;
  // Reset the in-hook peak accumulator: the instrumented subclass samples
  // `usedJSHeapSize` from inside `connectedCallback` / `$activateDeferredSSR`
  // (after stopping its CPU timer, so sampling never inflates cpuMs), giving a
  // truthful "peak while committing" figure rather than one taken only after the
  // debug event and scaffold cleanup.
  win.__benchPeakHeap = 0;
  const longTasks: number[] = [];
  const observer = installLongTaskObserver(longTasks);
  const baseHeap = sampleHeap();
  let peakHeap = baseHeap;
  const notePeak = (): void => {
    const h = sampleHeap();
    if (h !== null && (peakHeap === null || h > peakHeap)) peakHeap = h;
    // Fold in the max the instrumented hooks observed mid-hydration.
    const inHook = win.__benchPeakHeap ?? 0;
    if (inHook > 0 && (peakHeap === null || inHook > peakHeap)) peakHeap = inHook;
  };

  let elapsedMs: number;
  let receivedBoundaryLabels: string[] | undefined;

  if (config.mode === 'ordinary') {
    // Insert the SSR roots after the baseline heap sample so the
    // empty->populated transition matches the streaming arms.
    document.body.insertAdjacentHTML('beforeend', config.bodyHtml ?? '');
    notePeak();
    // Time only the synchronous hydration burst (define + upgrade), not the
    // one-time body parse above.
    const start = performance.now();
    win.__defineBenchIsland();
    elapsedMs = performance.now() - start;
    notePeak();
  } else {
    const boundaries = config.boundaries ?? [];
    if (config.captureBoundaryState) {
      win.__defineBenchIsland();
      receivedBoundaryLabels = [];
      const activate = Symbol.for('microsoft.webui.boundaryActivate');
      type Activation = (
        this: Element,
        state?: Record<string, unknown>,
      ) => void;
      const ctor = customElements.get('bench-island') as
        | (CustomElementConstructor & { prototype: Record<symbol, Activation> })
        | undefined;
      const original = ctor?.prototype[activate];
      if (!ctor || typeof original !== 'function') {
        throw new Error('bench: state-delivery probe could not wrap the activation hook');
      }
      const labels = receivedBoundaryLabels;
      ctor.prototype[activate] = function captureBoundaryState(
        this: Element,
        state?: Record<string, unknown>,
      ): void {
        labels.push(typeof state?.label === 'string' ? state.label : '<missing>');
        original.call(this, state);
      };
    }

    // Deterministic DOM-based synchronisation. The coordinator removes each
    // boundary's scaffolding (sentinel + payload/extension scripts + markers)
    // synchronously in its commit `finally`, driven by a single
    // `queueMicrotask` pump. So after inserting a boundary, spinning the
    // microtask queue until no `<webui-hydrate>` sentinel remains proves that
    // boundary committed and was cleaned — without depending on the opt-in
    // `webui:boundary-hydrated` diagnostic or the completion event (whose firing
    // is coupled to evolving coordinator internals). A bounded macrotask
    // fallback covers any deferred work; exceeding both bounds is a real hang
    // and throws rather than silently passing.
    const noSentinels = (): boolean =>
      document.getElementsByTagName('webui-hydrate').length === 0;
    const fullyClean = (): boolean =>
      noSentinels()
      && document.querySelectorAll('script[data-webui-boundary]').length === 0
      && document.querySelectorAll('[data-ws]').length === 0;
    const waitFor = async (predicate: () => boolean): Promise<void> => {
      for (let i = 0; i < 20_000; i++) {
        if (predicate()) return;
        await Promise.resolve();
      }
      for (let i = 0; i < 5_000; i++) {
        if (predicate()) return;
        await new Promise((resolve) => setTimeout(resolve, 0));
      }
      if (!predicate()) throw new Error('bench: streaming coordinator did not settle within bounds');
    };

    if (config.timing === 'eager') win.__defineBenchIsland();

    const start = performance.now();
    for (let seq = 0; seq < boundaries.length; seq++) {
      document.body.insertAdjacentHTML('beforeend', boundaries[seq]);
      await waitFor(noSentinels);
      if (config.timing === 'race' && seq === 0) win.__defineBenchIsland();
      notePeak();
    }
    document.body.insertAdjacentHTML('beforeend', config.terminal ?? '');
    // Wait until every root is activated (data-ws stripped, incl. late race
    // activations) and all boundary scaffolding is gone.
    await waitFor(fullyClean);
    elapsedMs = performance.now() - start;
    notePeak();
  }

  // Flush any pending longtask notifications before reading them.
  await new Promise((resolve) => setTimeout(resolve, 0));
  if (observer) {
    for (const entry of observer.takeRecords()) longTasks.push(entry.duration);
    observer.disconnect();
  }

  // Reactive probe (independent proof of real hydration). All timed/peak
  // measurements are already stopped above, so this never affects cpuMs,
  // elapsedMs, peak, or longtask. It runs before the caller's forced-GC retained
  // sample, but every arm performs the identical probe so the retained
  // comparison stays fair. For each root, drive its real public `setState` with a
  // shared sentinel `label`; `setState` flushes synchronously via the normal
  // API, so a genuinely hydrated root's bound `<span>` text must become the
  // sentinel. A non-hydrated root has no wired binding, so its text is unchanged.
  const REACTIVE_SENTINEL = '__bench_reactive_probe__';
  const islands = document.getElementsByTagName('bench-island');
  let verifiedReactiveCount = 0;
  for (let i = 0; i < islands.length; i++) {
    const el = islands[i] as unknown as {
      setState(state: Record<string, unknown>): void;
      querySelector(selector: string): Element | null;
    };
    el.setState({ label: REACTIVE_SENTINEL });
    const span = el.querySelector('span');
    if (span && span.textContent === REACTIVE_SENTINEL) verifiedReactiveCount++;
  }

  return {
    cpuMs: win.__benchHydrationCpu ?? 0,
    elapsedMs,
    peakHeapDeltaBytes:
      peakHeap !== null && baseHeap !== null ? peakHeap - baseHeap : null,
    maxLongTaskMs: longTasks.length ? Math.max(...longTasks) : null,
    liveRootCount: islands.length,
    hydratedCount: win.__benchHydratedCount ?? 0,
    verifiedReactiveCount,
    scaffoldScripts: document.querySelectorAll('script[data-webui-boundary]').length,
    scaffoldSentinels: document.getElementsByTagName('webui-hydrate').length,
    scaffoldComments: countBoundaryComments(),
    scaffoldDataWs: document.querySelectorAll('[data-ws]').length,
    streamedStatePopulated: !!(win.__webui && win.__webui.state !== undefined),
    receivedBoundaryLabels,
  };
}
