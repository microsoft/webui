// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export type LazyMode = 'eager' | 'visible';

export interface LazyRunMetrics {
  bundleInitMs: number;
  hydrationCpuMs: number;
  definitionMs: number;
  initialReadyMs: number;
  initialHydratedCount: number;
  initialListenerCount: number;
  initialPeakHeapDeltaBytes: number | null;
  maxLongTaskMs: number | null;
  visibleInteractionMs: number;
  dormantInteractionMs: number;
  dormantInteractionHydrated: boolean;
  interactionCount: number;
  liveRootCount: number;
}

export function insertLazyRoots(html: string): void {
  document.body.insertAdjacentHTML('beforeend', html);
}

/**
 * Drive one already-defined `bench-todo-item` fixture through hydration.
 * `mode` only affects the expected-count math below (which roots must hydrate
 * up front) — the fixture bundle passed to the page already baked in the
 * matching `hydration` strategy at build time (see `lazy-fixtures.ts`).
 */
export async function runLazyHydration(mode: LazyMode): Promise<LazyRunMetrics> {
  const win = window as unknown as {
    __benchBundleInitMs?: number;
    __benchHydrationCpu?: number;
    __benchHydratedCount?: number;
    __benchListenerCount?: number;
    __benchPeakHeap?: number;
    __benchInteractionCount?: number;
    __defineBenchTodo(): void;
  };

  function heapSize(): number | null {
    const memory = (performance as unknown as {
      memory?: { usedJSHeapSize: number };
    }).memory;
    return memory?.usedJSHeapSize ?? null;
  }

  const longTasks: Array<{ startTime: number; duration: number }> = [];
  let longTaskObserver: PerformanceObserver | null = null;
  try {
    longTaskObserver = new PerformanceObserver((list) => {
      const entries = list.getEntries();
      for (let i = 0; i < entries.length; i++) {
        longTasks.push({
          startTime: entries[i].startTime,
          duration: entries[i].duration,
        });
      }
    });
    longTaskObserver.observe({ type: 'longtask', buffered: true });
  } catch {
    longTaskObserver = null;
  }

  win.__benchHydrationCpu = 0;
  win.__benchHydratedCount = 0;
  win.__benchListenerCount = 0;
  win.__benchInteractionCount = 0;
  const baseHeap = heapSize();
  win.__benchPeakHeap = baseHeap ?? 0;
  const roots = document.getElementsByTagName('bench-todo-item');
  const viewportLimit = window.innerHeight + 200;
  let expectedInitialCount = 0;
  for (let i = 0; i < roots.length; i++) {
    if (roots[i].getBoundingClientRect().top > viewportLimit) break;
    expectedInitialCount++;
  }

  await new Promise((resolve) => setTimeout(resolve, 0));
  const started = performance.now();
  const definitionStarted = performance.now();
  win.__defineBenchTodo();
  const definitionMs = performance.now() - definitionStarted;

  for (
    let frame = 0;
    frame < 120 && (win.__benchHydratedCount ?? 0) < expectedInitialCount;
    frame++
  ) {
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  }
  const readyAt = performance.now();
  const initialReadyMs = readyAt - started;
  const initialHydratedCount = win.__benchHydratedCount ?? 0;
  const initialListenerCount = win.__benchListenerCount ?? 0;
  if (initialHydratedCount < expectedInitialCount) {
    throw new Error(
      `lazy hydration reached ${initialHydratedCount} of ${expectedInitialCount} expected initial roots`,
    );
  }
  const initialHydrationCpu = win.__benchHydrationCpu ?? 0;
  const peakHeap = Math.max(win.__benchPeakHeap ?? 0, heapSize() ?? 0);

  await new Promise((resolve) => setTimeout(resolve, 0));
  if (longTaskObserver) {
    const records = longTaskObserver.takeRecords();
    for (let i = 0; i < records.length; i++) {
      longTasks.push({
        startTime: records[i].startTime,
        duration: records[i].duration,
      });
    }
    longTaskObserver.disconnect();
  }

  const firstButton = roots[0]?.querySelector('button');
  const lastRoot = roots[roots.length - 1] as HTMLElement | undefined;
  const lastButton = lastRoot?.querySelector('button');
  if (!firstButton || !lastRoot || !lastButton) {
    throw new Error('lazy hydration benchmark roots are incomplete');
  }

  const visibleStarted = performance.now();
  firstButton.click();
  const visibleInteractionMs = performance.now() - visibleStarted;

  const beforeDormant = win.__benchHydratedCount ?? 0;
  const dormantStarted = performance.now();
  lastButton.click();
  const dormantInteractionMs = performance.now() - dormantStarted;
  const afterDormant = win.__benchHydratedCount ?? 0;

  return {
    bundleInitMs: win.__benchBundleInitMs ?? 0,
    hydrationCpuMs: initialHydrationCpu,
    definitionMs,
    initialReadyMs,
    initialHydratedCount,
    initialListenerCount,
    initialPeakHeapDeltaBytes:
      baseHeap === null ? null : peakHeap - baseHeap,
    maxLongTaskMs: (() => {
      let max = 0;
      for (let i = 0; i < longTasks.length; i++) {
        const task = longTasks[i];
        if (
          task.startTime < readyAt &&
          task.startTime + task.duration > started &&
          task.duration > max
        ) {
          max = task.duration;
        }
      }
      return max === 0 ? null : max;
    })(),
    visibleInteractionMs,
    dormantInteractionMs,
    dormantInteractionHydrated: afterDormant > beforeDormant,
    interactionCount: win.__benchInteractionCount ?? 0,
    liveRootCount: roots.length,
  };
}
