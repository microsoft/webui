// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TestRecursiveTree } from './element.js';
import type { FragmentCheckpoint } from './performance-stage-types.js';

export interface AdoptSample {
  bundleEvalMs: number;
  adoptSettleMs: number;
  adoptTotalMs: number;
  hydratedLeafUpdateMs: number;
  observedHeapDeltaBytes: number | null;
  ssrHtmlBytes: number;
  ssrItems: number;
  siblingHosts: number;
  adoptedItems: number;
  definedBeforeInjection: boolean;
  sameHostObject: boolean;
  adoptedIdentity: boolean;
  identityProbes: number;
  markupUnchanged: boolean;
  markupDeltaBytes: number;
  hydrations: number;
  ready: boolean;
  leafUpdated: boolean;
}

/** Staged cold adopts a detached, never-upgraded deep/16 SSR host after bundle injection. */
export interface AdoptInput {
  tag: string;
  source: string;
  lastId: string;
  staged?: 'cold' | 'warm';
  detachedHost?: Element;
}

/** Append activation is separate from the default bundle-evaluation/adoption timers. */
export interface StagedAdoptInfo {
  activationSyncMs: number;
  activationSettleMs: number;
  teardownSettled: boolean;
  bootstrapReleased: boolean;
}

/** The ordinary result has exactly the original AdoptSample fields. */
export type AdoptResult = AdoptSample & { staged?: StagedAdoptInfo };

/**
 * SSR scenario: the page already holds real server-rendered markup and no
 * framework code. Injecting the current bundle defines the tags, which upgrades
 * the parsed hosts - that upgrade is the hydration cost being timed.
 */
export async function adoptScenario(input: AdoptInput): Promise<AdoptResult> {
  if (input.staged === 'warm') {
    throw new Error(
      'Warm ordinary SSR is blocked by the one-shot buffered SSR bootstrap contract: ' +
      'non-routed webui:hydration-complete synchronously deletes window.__webui.state ' +
      'after the first completed host; the detached, unupgraded second host has not primed its roots. ' +
      'loadWebUIDataBlock is latched and #webui-data is removed. ' +
      'setState on an unmounted authored host cannot establish fragment known-root provenance. ' +
      'Report this lane as blocked; do not restore state, suppress completion, fake route metadata, ' +
      'use hooks, or substitute client creation. A future real two-boundary streamed SSR lane ' +
      'needs independently compiler-emitted boundary state; it is not implemented here.',
    );
  }
  const staged = input.staged === 'cold';
  const checkpoint = staged ? window.__webuiFragmentCheckpoint : undefined;
  if (staged && typeof checkpoint !== 'function') {
    throw new Error('Staged SSR scenario requires window.__webuiFragmentCheckpoint before activation');
  }
  if (staged && !input.detachedHost) {
    throw new Error('Staged cold SSR requires detachedHost from the real server-rendered document');
  }
  const { tag, source, lastId } = input;
  const host = staged ? input.detachedHost : document.querySelector(tag);
  if (!host) throw new Error(`SSR host <${tag}> missing`);
  const rootOf = (node: Element): Element | ShadowRoot => node.shadowRoot ?? node;

  function heap(): number | null {
    const memory = (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory;
    return memory?.usedJSHeapSize ?? null;
  }

  const encoder = new TextEncoder();
  const ssrRoot = rootOf(host);
  const ssrNodes = Array.from(ssrRoot.querySelectorAll<HTMLLIElement>('li[data-id]'));
  const ssrItems = ssrNodes.length;
  const ssrMarkup = ssrRoot.innerHTML;
  const ssrHtmlBytes = encoder.encode(ssrMarkup).length;
  const definedBeforeInjection = customElements.get(tag) !== undefined;
  const siblingHosts = document.querySelectorAll(
    'test-recursive-light,test-recursive-table,test-recursive-unknown,test-recursive-lazy',
  ).length;
  const step = Math.max(1, Math.floor(ssrItems / 32));
  const probes: Array<{ id: string; node: Element }> = [];
  for (let index = 0; index < ssrNodes.length; index += step) {
    probes.push({ id: ssrNodes[index].dataset.id ?? '', node: ssrNodes[index] });
  }
  ssrNodes.length = 0;
  let preAppendEvidence: FragmentCheckpoint['evidence'];
  let expectedLeafName: string | undefined;
  let expectedTitle: string | undefined;
  if (staged) {
    const definition = customElements.get(tag);
    if (!definition) {
      throw new Error('Inject the bundle into the host-free document before staged cold SSR');
    }
    const internals = host as Element & { $ready?: boolean; hydrations?: number };
    const preAppendUpgraded = host instanceof definition || host.matches(':defined');
    const preAppendReady = internals.$ready ?? null;
    if (
      tag !== 'test-recursive-tree' || host.localName !== tag ||
      host.ownerDocument !== document || host.parentNode !== null || host.isConnected ||
      preAppendUpgraded || preAppendReady !== null || internals.hydrations !== undefined ||
      document.querySelector(tag) !== null || siblingHosts !== 0 || host.hasAttribute('data-ws')
    ) {
      throw new Error(
        'Staged cold SSR requires a host-free document and its detached, unupgraded, not-ready ' +
        'ordinary <test-recursive-tree>; do not upgrade, create, or assign state to it in the driver',
      );
    }
    const runtime = window.__webui;
    if (
      runtime?.state === undefined || runtime.state === null ||
      runtime.chain !== undefined || runtime.templateHostExclusions !== undefined
    ) {
      throw new Error(
        'Staged cold SSR requires unconsumed window.__webui.state with no chain or ' +
        'templateHostExclusions; do not restore bootstrap state or fake routing metadata',
      );
    }
    const comments = document.createTreeWalker(ssrRoot, NodeFilter.SHOW_COMMENT);
    let fragmentOpenMarkers = 0;
    let fragmentCloseMarkers = 0;
    let capturedMarkers = 0;
    for (let node = comments.nextNode(); node; node = comments.nextNode()) {
      if (node.nodeValue === 'wf') fragmentOpenMarkers++;
      if (node.nodeValue === '/wf') fragmentCloseMarkers++;
      if (node.nodeValue?.startsWith('wf:')) capturedMarkers++;
    }
    if (
      host.shadowRoot === null || !ssrRoot.hasChildNodes() || ssrItems !== 16 ||
      fragmentOpenMarkers + capturedMarkers === 0 ||
      fragmentOpenMarkers + capturedMarkers !== fragmentCloseMarkers
    ) {
      throw new Error(
        'Staged cold SSR requires genuine preexisting shadow DOM, 16 server items, and paired ' +
        'wf or wf:ID markers from the real deep/16 render; no client-created substitute',
      );
    }
    let stateItems: unknown = runtime.state.items;
    let stateDepth = 0;
    while (Array.isArray(stateItems) && stateItems.length === 1 && stateDepth < 16) {
      const item: unknown = stateItems[0];
      if (
        typeof item !== 'object' || item === null ||
        !('id' in item) || item.id !== `node-${stateDepth}` ||
        !('name' in item) || typeof item.name !== 'string' || !('children' in item)
      ) break;
      expectedLeafName = item.name;
      stateItems = item.children;
      stateDepth++;
    }
    if (
      stateDepth !== 16 || !Array.isArray(stateItems) || stateItems.length !== 0 ||
      lastId !== 'node-15' || typeof runtime.state.title !== 'string' ||
      ssrRoot.querySelector(`[data-id="${lastId}"] > .name`)?.textContent !== expectedLeafName ||
      ssrRoot.querySelector('h2')?.textContent !== runtime.state.title
    ) {
      throw new Error('Staged cold SSR requires matching ordinary reflected deep/16 bootstrap state and server markup');
    }
    expectedTitle = runtime.state.title;
    preAppendEvidence = {
      shape: 'deep',
      size: 16,
      tagDefined: definedBeforeInjection,
      preAppendUpgraded,
      preAppendReady,
      preAppendConnected: host.isConnected,
      preexistingShadowRoot: host.shadowRoot !== null,
      nonemptyServerNodes: ssrRoot.hasChildNodes(),
      ssrItems,
      ssrHtmlBytes,
      fragmentOpenMarkers,
      fragmentCloseMarkers,
      fragmentMarkerCount: fragmentOpenMarkers + capturedMarkers + fragmentCloseMarkers,
      capturedMarkers,
      bootstrapPresent: true,
      bootstrapStateDepth: stateDepth,
      routeMetadataAbsent: true,
    };
  }
  if (checkpoint) {
    await checkpoint({ phase: 'input-ready', lane: 'ssr', evidence: preAppendEvidence });
  }
  const beforeHeap = heap();

  let measuredBundleEvalMs: number;
  let measuredAdoptSettleMs: number;
  let adoptedElement: TestRecursiveTree;
  let activationSyncMs = 0;
  let activationSettleMs = 0;
  if (staged) {
    if (
      window.__webui?.state === undefined || window.__webui.state === null ||
      window.__webui.chain !== undefined || window.__webui.templateHostExclusions !== undefined
    ) {
      throw new Error('Cold SSR bootstrap became unavailable or routed during input-ready; activation refused');
    }
    const activationStart = performance.now();
    document.body.append(host);
    activationSyncMs = performance.now() - activationStart;

    const activationSettleStart = performance.now();
    await new Promise<void>(resolve => queueMicrotask(resolve));
    const element = document.querySelector(tag) as TestRecursiveTree | null;
    if (!element) throw new Error(`<${tag}> vanished during adoption`);
    element.$flushUpdates();
    await new Promise<void>(resolve => setTimeout(resolve, 0));
    activationSettleMs = performance.now() - activationSettleStart;
    measuredBundleEvalMs = 0;
    measuredAdoptSettleMs = activationSettleMs;
    adoptedElement = element;
  } else {
    const script = document.createElement('script');
    script.textContent = source;
    const evalStart = performance.now();
    document.head.append(script);
    const bundleEvalMs = performance.now() - evalStart;

    const settleStart = performance.now();
    await new Promise<void>(resolve => queueMicrotask(resolve));
    const element = document.querySelector(tag) as TestRecursiveTree | null;
    if (!element) throw new Error(`<${tag}> vanished during adoption`);
    element.$flushUpdates();
    await new Promise<void>(resolve => setTimeout(resolve, 0));
    const adoptSettleMs = performance.now() - settleStart;
    measuredBundleEvalMs = bundleEvalMs;
    measuredAdoptSettleMs = adoptSettleMs;
    adoptedElement = element;
  }
  const bundleEvalMs = measuredBundleEvalMs;
  const adoptSettleMs = measuredAdoptSettleMs;
  const element = adoptedElement;

  const observedHeap = heap();
  const adoptedRoot = rootOf(element);
  const adoptedMarkup = adoptedRoot.innerHTML;
  const adoptedItems = adoptedRoot.querySelectorAll('li[data-id]').length;
  const adoptedIdentity = probes.every(probe => {
    const current = adoptedRoot.querySelector(`li[data-id="${probe.id}"]`);
    return current === probe.node && probe.node.isConnected;
  });
  const identityProbes = probes.length;
  probes.length = 0;
  const hydrations = element.hydrations;
  const ready = (element as unknown as { $ready: boolean }).$ready === true;

  // The adopted tree must be live, not just correct markup: mutate the deepest
  // (or last) item through the hydrated JS state and confirm the DOM follows.
  let leaf = element.items[element.items.length - 1];
  while (leaf && leaf.children.length) leaf = leaf.children[leaf.children.length - 1];
  if (checkpoint) {
    const sameHostObject = element === (host as unknown as TestRecursiveTree);
    const sameShadowRoot = element.shadowRoot === ssrRoot;
    const liveStateVerified =
      element.items.length === 1 && element.items[0].id === 'node-0' &&
      element.title === expectedTitle && leaf?.name === expectedLeafName;
    const deepestLeafVerified =
      leaf?.id === lastId && leaf.children.length === 0 &&
      adoptedRoot.querySelector(`[data-id="${lastId}"] > .name`)?.textContent === expectedLeafName;
    const bootstrapReleased = window.__webui?.state === undefined;
    if (
      !sameHostObject || !sameShadowRoot || !adoptedIdentity || identityProbes === 0 ||
      adoptedItems !== ssrItems || hydrations !== 1 || !ready ||
      !liveStateVerified || !deepestLeafVerified || !bootstrapReleased
    ) {
      throw new Error(
        'Cold SSR adoption failed server-node identity, single hydration, readiness, live deep/16 state, ' +
        'or one-shot bootstrap release; report the failed SSR contract, not a client fallback',
      );
    }
    await checkpoint({
      phase: 'mounted',
      lane: 'ssr',
      evidence: {
        ...preAppendEvidence,
        sameHostObject,
        sameShadowRoot,
        adoptedIdentity,
        identityProbes,
        adoptedItems,
        hydrations,
        ready,
        liveStateVerified,
        deepestLeafVerified,
        deepestLeafId: leaf.id,
        bootstrapReleased,
      },
      timings: { activationSyncMs, activationSettleMs, bundleEvalMs, adoptSettleMs },
    });
  }
  const updateStart = performance.now();
  if (leaf) leaf.name = 'Adopted leaf';
  element.$update();
  element.$flushUpdates();
  const hydratedLeafUpdateMs = performance.now() - updateStart;
  const leafUpdated =
    adoptedRoot.querySelector(`[data-id="${lastId}"] > .name`)?.textContent === 'Adopted leaf';

  let stagedInfo: StagedAdoptInfo | undefined;
  if (staged) {
    const internals = element as unknown as { $ready: boolean; $root: unknown };
    element.remove();
    await new Promise<void>(resolve => queueMicrotask(resolve));
    await new Promise<void>(resolve => setTimeout(resolve, 0));
    const teardownSettled =
      !element.isConnected && internals.$ready === false && internals.$root === null;
    const bootstrapReleased = window.__webui?.state === undefined;
    if (!leafUpdated || !teardownSettled || !bootstrapReleased) {
      throw new Error('Cold SSR leaf reactivity, settled teardown, and bootstrap release must all succeed');
    }
    stagedInfo = { activationSyncMs, activationSettleMs, teardownSettled, bootstrapReleased };
  }
  const sample: AdoptResult = {
    bundleEvalMs,
    adoptSettleMs,
    adoptTotalMs: bundleEvalMs + adoptSettleMs,
    hydratedLeafUpdateMs,
    observedHeapDeltaBytes: beforeHeap === null || observedHeap === null
      ? null
      : observedHeap - beforeHeap,
    ssrHtmlBytes,
    ssrItems,
    siblingHosts,
    adoptedItems,
    definedBeforeInjection,
    sameHostObject: element === (host as unknown as TestRecursiveTree),
    adoptedIdentity,
    identityProbes,
    markupUnchanged: adoptedMarkup === ssrMarkup,
    markupDeltaBytes: encoder.encode(adoptedMarkup).length - ssrHtmlBytes,
    hydrations,
    ready,
    leafUpdated,
  };
  // The caller must await this return and release detachedHost before cleanup heap measurement.
  if (stagedInfo) sample.staged = stagedInfo;
  return sample;
}
