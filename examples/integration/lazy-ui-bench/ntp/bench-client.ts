// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * In-page side of the NTP benchmark, shared by every variant.
 *
 * Owners register when they hydrate. The driver arms a measurement, sends real
 * pointer input, and reads when the UI became visually complete: every
 * sentinel element is connected, visible, laid out, and styled by its own
 * stylesheet (`--ntp-ready: 1`, a non-inherited registered property).
 */

/** One on-demand UI as the harness drives it. */
export interface BenchUi {
  /** Owner tag; owners are unique on the page. */
  owner: string;
  /** Trigger selector inside the owner's shadow root. */
  trigger?: string;
  /** Selector paths from the owner, each step entering the previous match's shadow root. */
  sentinels: string[][];
  /** Custom elements the UI needs defined. */
  tags: string[];
  /** Lazy region identity, `<owner>::lazy:@N`. */
  region?: string;
  /** Component template the UI needs registered (component assets). */
  template?: string;
  /** Owner state path of the condition. */
  cond: string;
  /** Owner methods. */
  open: string;
  close: string;
  preload?: string;
}

/** Fixture facts the driver passes at startup. */
export interface BenchConfig {
  variant: string;
  owners: string[];
  uis: Record<string, BenchUi>;
}

type ResourceRow = [name: string, start: number, end: number, transfer: number, initiator: string];

/** One measured open, in page milliseconds. */
export interface OpenResult {
  id: string;
  armedAt: number;
  inputAt: number;
  visibleAt: number;
  paintedAt: number;
  resources: ResourceRow[];
  longTasks: [number, number][];
}

/** Startup facts recorded by the driver's init script. */
interface PerfLog {
  longTasks: [number, number][];
  fcp: number;
  lcp: number;
  lcpUrl: string;
  lcpSize: number;
  lcpElement: string;
  resolveReady(): void;
}

type Owner = HTMLElement & Record<string, unknown> & {
  $flushUpdates?(): void;
  $prefetch?(path?: string): void;
};

declare global {
  interface Window {
    __perf?: PerfLog;
    __bench?: typeof bench;
  }
}

/** Templates the page has registered, read without depending on the framework's global types. */
function registered(): Record<string, unknown> {
  return (window as unknown as { __webui?: { templates?: Record<string, unknown> } }).__webui?.templates ?? {};
}

const TIMEOUT_MS = 30_000;
const owners = new Map<string, Owner>();
let config: BenchConfig = { variant: '', owners: [], uis: {} };
let hydratedAt = 0;
let armed: { id: string; armedAt: number; inputAt: number } | undefined;
let result: OpenResult | undefined;
let failure = '';
let waiting: (() => void) | undefined;

function settle(): void {
  const resolve = waiting;
  waiting = undefined;
  resolve?.();
}

function call(owner: Owner, method: string, ...args: unknown[]): unknown {
  const fn = owner[method];
  if (typeof fn !== 'function') throw new Error('<' + owner.localName + '> has no method ' + method);
  return (fn as (...values: unknown[]) => unknown).apply(owner, args);
}

function ownerOf(id: string): Owner {
  const ui = config.uis[id];
  const owner = ui && owners.get(ui.owner);
  if (!owner) throw new Error('No hydrated owner for ' + id);
  return owner;
}

function find(owner: Element, path: readonly string[]): Element | null {
  let node: Element | null = owner;
  for (const selector of path) {
    const root: ParentNode = node.shadowRoot ?? node;
    node = root.querySelector(selector);
    if (!node) return null;
  }
  return node;
}

/** Every sentinel is connected, visible, laid out, and styled by its component's stylesheet. */
function complete(id: string): boolean {
  const ui = config.uis[id];
  const owner = owners.get(ui.owner);
  if (!owner) return false;
  for (const path of ui.sentinels) {
    const node = find(owner, path);
    if (!node?.isConnected) return false;
    if (!node.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false;
    const rect = node.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return false;
    if (getComputedStyle(node).getPropertyValue('--ntp-ready').trim() !== '1') return false;
  }
  return true;
}

function until(check: () => boolean): Promise<void> {
  return new Promise((resolve, reject) => {
    const start = performance.now();
    const tick = (): void => {
      if (check()) resolve();
      else if (performance.now() - start > TIMEOUT_MS) reject(new Error('Timed out'));
      else requestAnimationFrame(tick);
    };
    tick();
  });
}

function resourcesSince(time: number): ResourceRow[] {
  const rows: ResourceRow[] = [];
  for (const entry of performance.getEntriesByType('resource') as PerformanceResourceTiming[]) {
    if (entry.startTime >= time - 1) {
      rows.push([entry.name, entry.startTime, entry.responseEnd, entry.transferSize, entry.initiatorType]);
    }
  }
  return rows;
}

function watch(): void {
  const current = armed;
  if (!current) return;
  const tick = (): void => {
    if (armed !== current) return;
    if (complete(current.id)) {
      const visibleAt = performance.now();
      // The frame that found the UI complete paints before the next task runs.
      setTimeout(() => {
        const paintedAt = performance.now();
        result = {
          ...current, visibleAt, paintedAt,
          resources: resourcesSince(current.armedAt),
          longTasks: (window.__perf?.longTasks ?? []).filter(([start, duration]) => start + duration > current.inputAt),
        };
        armed = undefined;
        settle();
      }, 0);
      return;
    }
    if (performance.now() - current.inputAt > TIMEOUT_MS) {
      failure = 'Timed out waiting for ' + current.id + ' to become visually complete';
      armed = undefined;
      settle();
      return;
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

document.addEventListener('pointerdown', event => {
  if (armed && !armed.inputAt) {
    armed.inputAt = event.timeStamp;
    watch();
  }
}, true);

export const bench = {
  configure(value: BenchConfig): void {
    config = value;
  },

  /** Owners call this from `hydratedCallback()`. */
  register(element: HTMLElement): void {
    owners.set(element.localName, element as Owner);
    if (hydratedAt || owners.size < config.owners.length) return;
    hydratedAt = performance.now();
    const ready = (): void => window.__perf?.resolveReady();
    if (document.readyState === 'complete') ready();
    else window.addEventListener('load', ready, { once: true });
  },

  get hydratedAt(): number {
    return hydratedAt;
  },

  /** Start a measurement that the next pointerdown begins. */
  arm(id: string): void {
    armed = { id, armedAt: performance.now(), inputAt: 0 };
    result = undefined;
    failure = '';
  },

  /** Resolve with the armed measurement once it completes. */
  async result(): Promise<OpenResult | { error: string }> {
    if (!result && !failure) await new Promise<void>(resolve => { waiting = resolve; });
    return result ?? { error: failure };
  },

  /** Open from code, as the primer and the memory cycles do; resolves when visually complete. */
  async open(id: string): Promise<number> {
    const start = performance.now();
    await call(ownerOf(id), config.uis[id].open);
    await until(() => complete(id));
    return performance.now() - start;
  },

  close(id: string): void {
    const owner = ownerOf(id);
    call(owner, config.uis[id].close);
    owner.$flushUpdates?.();
  },

  /** Load a UI's code and templates ahead of use: `$prefetch` or the component-asset loader. */
  prefetch(id: string): void {
    const ui = config.uis[id];
    const owner = ownerOf(id);
    if (config.variant === 'lazy') owner.$prefetch?.(ui.cond);
    else if (ui.preload) call(owner, ui.preload);
  },

  /** Whether everything the UI needs is registered and defined. */
  prefetched(id: string): boolean {
    const ui = config.uis[id];
    const templates = registered();
    if (ui.region && !templates[ui.region]) return false;
    if (ui.template && !templates[ui.template]) return false;
    return ui.tags.every(tag => customElements.get(tag) !== undefined);
  },

  complete,

  /** Viewport center of a UI's trigger, for real pointer input. */
  point(id: string): { x: number; y: number } | null {
    const ui = config.uis[id];
    const owner = owners.get(ui.owner);
    const target = ui.trigger && owner ? find(owner, [ui.trigger]) : null;
    if (!target) return null;
    const rect = target.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  },

  /** Lazy regions whose templates are registered now. */
  loadedRegions(): number {
    const templates = registered();
    return Object.values(config.uis).filter(ui => ui.region && templates[ui.region]).length;
  },

  /** Where focus is, through shadow roots, for the autofocus check. */
  focusPath(): string {
    const path: string[] = [];
    let node: Element | null = document.activeElement;
    while (node) {
      path.push(node.localName);
      node = node.shadowRoot?.activeElement ?? null;
    }
    return path.join(' > ');
  },
};

window.__bench = bench;
