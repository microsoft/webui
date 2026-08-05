// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import '../../../src/visible-hydration-entry.js';
import { attr, observable, WebUIElement } from '../../../src/index.js';
import {
  disconnectLazyHydration,
  LAZY_HYDRATION_ACTIVATE,
  observeLazyHydration,
} from '../../../src/lazy-hydration.js';
import {
  __getLifecycleStateForTests,
  __resetLifecycleForTests,
} from '../../../src/lifecycle.js';
import {
  isStreamingHydrationMode,
  resetStreamingModeForTests,
} from '../../../src/streaming-mode.js';

declare global {
  interface Window {
    __lazyHydrationLog?: string[];
    __shadowProbeChild?: HTMLElement;
    __slowLazyProbes?: LazyProbe[];
    __slowLazyActivated?: number;
    __failureLazyProbes?: HTMLElement[];
    __healthyLazyActivated?: number;
    __nestedSlowDeepest?: HTMLElement;
    __createShadowLazyProbe?: () => void;
    __createSlowLazyProbes?: (count: number) => void;
    __createNestedSlowLazyProbes?: (count: number) => void;
    __createFailureLazyProbes?: () => void;
    __reconnectSlowLazyProbe?: (index: number) => void;
    __enableStreamingModeForTest?: () => boolean;
    __lazyHydrationPendingCount?: () => number;
    __resetLazyHydrationLifecycle?: () => void;
  }
}

type LazyProbe = HTMLElement & {
  [LAZY_HYDRATION_ACTIVATE](
    state: Record<string, unknown> | undefined,
  ): void;
};

function createLazyProbe(id: string, activate: () => void): LazyProbe {
  const probe = Object.assign(document.createElement('div'), {
    [LAZY_HYDRATION_ACTIVATE](
      _state: Record<string, unknown> | undefined,
    ): void {
      activate();
    },
  });
  probe.id = id;
  return probe;
}

export class TestLazyItem extends WebUIElement {
  static override readonly hydration = 'visible';

  @attr label = '';
  @attr count = 0;
  @observable note = '';
  @observable sameValue = '';
  @observable focusCount = 0;

  protected override hydratedCallback(): void {
    this.setAttribute('data-hydrated', '');
    (window.__lazyHydrationLog ??= []).push(this.id);
  }

  increment(): void {
    this.count = Number(this.count) + 1;
  }

  recordFocus(): void {
    this.focusCount++;
  }
}
TestLazyItem.define('test-lazy-item');

export class TestLazyParent extends WebUIElement {
  static override readonly hydration = 'visible';

  @attr label = '';

  protected override hydratedCallback(): void {
    this.setAttribute('data-hydrated', '');
    (window.__lazyHydrationLog ??= []).push(this.id);
  }
}
TestLazyParent.define('test-lazy-parent');

interface LazyListItem {
  id: number;
  label: string;
  count: number;
}

export class TestLazyList extends WebUIElement {
  static override readonly hydration = 'visible';

  @observable items: LazyListItem[] = [];
  @observable showSummary = true;
  @observable summary = '';

  protected override hydratedCallback(): void {
    this.setAttribute('data-hydrated', '');
    (window.__lazyHydrationLog ??= []).push(this.id);
  }

  replaceItems(): void {
    this.items = this.items.map((item) => ({
      ...item,
      label: `Updated ${item.id}`,
      count: item.count + 10,
    }));
  }

  toggleSummary(): void {
    this.showSummary = !this.showSummary;
  }
}
TestLazyList.define('test-lazy-list');

window.__lazyHydrationPendingCount = (): number =>
  __getLifecycleStateForTests().pendingCount;
window.__resetLazyHydrationLifecycle = __resetLifecycleForTests;

window.__enableStreamingModeForTest = (): boolean => {
  let marker = document.querySelector(
    'meta[name="webui-streaming"][content="1"]',
  );
  if (!marker) {
    marker = document.createElement('meta');
    marker.setAttribute('name', 'webui-streaming');
    marker.setAttribute('content', '1');
    document.head.appendChild(marker);
  }
  resetStreamingModeForTests();
  return isStreamingHydrationMode();
};

window.__createShadowLazyProbe = (): void => {
  const order = window.__lazyHydrationLog ??= [];
  const parent = createLazyProbe('shadow-probe-parent', () => {
    order.push('shadow-probe-parent');
  });
  const child = createLazyProbe('shadow-probe-child', () => {
    order.push('shadow-probe-child');
  });
  const shadowHost = document.createElement('div');
  const slot = document.createElement('slot');
  parent.appendChild(slot);
  shadowHost.attachShadow({ mode: 'open' }).appendChild(parent);
  shadowHost.appendChild(child);
  document.body.appendChild(shadowHost);
  observeLazyHydration(parent);
  observeLazyHydration(child);
  window.__shadowProbeChild = child;
};

window.__createSlowLazyProbes = (count: number): void => {
  const probes = new Array<LazyProbe>(count);
  for (let i = 0; i < count; i++) {
    const probe = createLazyProbe(`slow-probe-${i}`, () => {
      const started = performance.now();
      while (performance.now() - started < 1) {
        // Intentional test-only work to exercise the coordinator's yield budget.
      }
      window.__slowLazyActivated = (window.__slowLazyActivated ?? 0) + 1;
    });
    document.body.appendChild(probe);
    observeLazyHydration(probe);
    probes[i] = probe;
  }
  window.__slowLazyProbes = probes;
};

window.__createNestedSlowLazyProbes = (count: number): void => {
  const probes = new Array<LazyProbe>(count);
  let root: LazyProbe | undefined;
  let parent: LazyProbe | undefined;
  for (let i = 0; i < count; i++) {
    const probe = createLazyProbe(`nested-slow-probe-${i}`, () => {
      const started = performance.now();
      while (performance.now() - started < 1) {
        // Intentional test-only work to exercise nested activation yielding.
      }
      window.__slowLazyActivated = (window.__slowLazyActivated ?? 0) + 1;
      (window.__lazyHydrationLog ??= []).push(probe.id);
    });
    if (parent) parent.appendChild(probe);
    else root = probe;
    parent = probe;
    probes[i] = probe;
  }
  if (!root || !parent) return;
  document.body.appendChild(root);
  for (let i = 0; i < probes.length; i++) {
    observeLazyHydration(probes[i]);
  }
  window.__slowLazyProbes = probes;
  window.__nestedSlowDeepest = parent;
};

window.__reconnectSlowLazyProbe = (index: number): void => {
  const probe = window.__slowLazyProbes?.[index];
  if (!probe) return;
  disconnectLazyHydration(probe);
  probe.remove();
  document.body.appendChild(probe);
  observeLazyHydration(probe);
};

window.__createFailureLazyProbes = (): void => {
  const broken = createLazyProbe('broken-probe', () => {
    throw new Error('intentional lazy activation failure');
  });
  const healthy = createLazyProbe('healthy-probe', () => {
    window.__healthyLazyActivated =
      (window.__healthyLazyActivated ?? 0) + 1;
  });
  broken.appendChild(healthy);
  document.body.appendChild(broken);
  observeLazyHydration(broken);
  observeLazyHydration(healthy);
  window.__failureLazyProbes = [broken, healthy];
};
