// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TestRecursiveTree, TreeItem } from './element.js';
import type { FragmentCheckpoint } from './performance-stage-types.js';

export interface Scenario {
  shape: 'deep' | 'wide';
  size: number;
  /** Opt in to harness checkpoints without changing the ordinary lifecycle timers. */
  staged?: boolean;
}

export interface Sample {
  mountMs: number;
  leafUpdateMs: number;
  ownerUpdateMs: number;
  replaceRootMs: number;
  noOpMs: number;
  reorderMs: number | null;
  insertionMs: number;
  deepReorderMs: number | null;
  structuralEditsValid: boolean;
  removeMs: number;
  reinsertMs: number;
  disconnectSyncMs: number;
  disconnectSettleMs: number;
  observedHeapDeltaBytes: number | null;
  htmlBytes: number;
  mountedItems: number;
  removedItems: number;
  reinsertedItems: number;
  retainedIdentity: boolean;
  leafUpdated: boolean;
  ownerUpdated: boolean;
  noOpMutations: number;
  hydrations: number;
  markupParity: boolean;
  markupParityDeltaBytes: number;
  teardownSettled: boolean;
}

/** Client-side scenario: creation, updates, reorder, remove/reinsert, teardown. */
export async function runScenario(scenario: Scenario): Promise<Sample> {
  function makeItems(suffix: string): TreeItem[] {
    let items: TreeItem[] = [];
    if (scenario.shape === 'wide') {
      for (let index = 0; index < scenario.size; index++) {
        items.push({ id: `node-${index}`, name: `Node ${index}${suffix}`, children: [] });
      }
    } else {
      for (let index = scenario.size - 1; index >= 0; index--) {
        items = [{ id: `node-${index}`, name: `Node ${index}${suffix}`, children: items }];
      }
    }
    return items;
  }

  function heap(): number | null {
    const memory = (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory;
    return memory?.usedJSHeapSize ?? null;
  }

  let observedHeap = heap();
  const beforeHeap = observedHeap;
  function measure(work: () => void): number {
    const start = performance.now();
    work();
    const elapsed = performance.now() - start;
    const currentHeap = heap();
    if (currentHeap !== null) observedHeap = Math.max(observedHeap ?? currentHeap, currentHeap);
    return elapsed;
  }

  const items = makeItems('');
  const checkpoint = scenario.staged ? window.__webuiFragmentCheckpoint : undefined;
  if (scenario.staged && typeof checkpoint !== 'function') {
    throw new Error('Staged client scenario requires window.__webuiFragmentCheckpoint');
  }
  if (checkpoint) {
    await checkpoint({
      phase: 'input-ready',
      lane: 'client',
      evidence: { shape: scenario.shape, size: scenario.size },
    });
  }
  let preAppendEvidence: FragmentCheckpoint['evidence'];
  const activationStart = checkpoint ? performance.now() : 0;
  const element = document.createElement('test-recursive-tree') as TestRecursiveTree;
  element.title = 'Fragment performance';
  element.items = items;
  if (checkpoint) {
    const preAppendEmpty = !element.hasChildNodes();
    const preAppendHasShadowRoot = element.shadowRoot !== null;
    const preAppendReady = (element as unknown as { $ready: boolean }).$ready;
    if (!preAppendEmpty || preAppendHasShadowRoot || preAppendReady !== false) {
      throw new Error('Staged client host must be empty, without a shadow root, and not ready before append');
    }
    preAppendEvidence = { preAppendEmpty, preAppendHasShadowRoot, preAppendReady };
  }
  const mountMs = measure(() => {
    document.body.append(element);
    element.$flushUpdates();
  });
  if (checkpoint) {
    const activationSyncMs = performance.now() - activationStart;
    const activationSettleStart = performance.now();
    await new Promise<void>(resolve => queueMicrotask(resolve));
    await new Promise<void>(resolve => setTimeout(resolve, 0));
    const activationSettleMs = performance.now() - activationSettleStart;
    const stagedRoot = element.shadowRoot ?? element;
    await checkpoint({
      phase: 'mounted',
      lane: 'client',
      evidence: {
        shape: scenario.shape,
        size: scenario.size,
        ...preAppendEvidence,
        ready: (element as unknown as { $ready: boolean }).$ready === true,
        hydrations: element.hydrations,
        mountedItems: stagedRoot.querySelectorAll('li[data-id]').length,
      },
      timings: { activationSyncMs, activationSettleMs, mountMs },
    });
  }
  const root = element.shadowRoot ?? element;
  const originalNodes = Array.from(root.querySelectorAll<HTMLLIElement>('li[data-id]'));
  const mountedItems = originalNodes.length;
  const encoder = new TextEncoder();
  const htmlBytes = encoder.encode(root.innerHTML).length;
  const lastId = `node-${scenario.size - 1}`;
  let leaf = scenario.shape === 'wide' ? items[items.length - 1] : items[0];
  while (leaf.children.length) leaf = leaf.children[0];
  leaf.name = 'Updated leaf';
  const leafUpdateMs = measure(() => {
    element.$update();
    element.$flushUpdates();
  });
  const leafUpdated = root.querySelector(`[data-id="${lastId}"] > .name`)?.textContent === 'Updated leaf';
  const ownerUpdateMs = measure(() => {
    element.title = 'Updated owner';
    element.$flushUpdates();
  });
  const ownerUpdated = root.querySelector('h2')?.textContent === 'Updated owner';
  const replacement = makeItems(' replaced');
  const replaceRootMs = measure(() => {
    element.items = replacement;
    element.$flushUpdates();
  });
  // Markup produced by an incremental root replacement. The remove/reinsert cycle
  // below rebuilds the identical state from empty, so any divergence between the
  // incremental and from-scratch projection shows up as a byte difference here.
  const parityReference = root.innerHTML;
  const noOpMs = measure(() => {
    element.$update();
    element.$flushUpdates();
  });

  // Keep mutation instrumentation out of the timed update paths.
  const observer = new MutationObserver(() => {});
  observer.observe(root, { childList: true, subtree: true, attributes: true, characterData: true });
  element.$update();
  element.$flushUpdates();
  const noOpMutations = observer.takeRecords().length;
  observer.disconnect();

  let reorderMs: number | null = null;
  if (scenario.shape === 'wide') {
    const reordered = replacement.toReversed();
    reorderMs = measure(() => {
      element.items = reordered;
      element.$flushUpdates();
    });
  }
  let siblings = element.items;
  if (scenario.shape === 'deep') {
    for (let depth = 0; depth < Math.floor(scenario.size / 2); depth++) {
      siblings = siblings[0].children;
    }
  }
  const insertionIndex = Math.floor(siblings.length / 2);
  const inserted: TreeItem = { id: 'inserted', name: 'Inserted', children: [] };
  siblings.splice(insertionIndex, 0, inserted);
  const insertionMs = measure(() => {
    element.$update();
    element.$flushUpdates();
  });
  const insertedNode = root.querySelector('li[data-id="inserted"]');
  let structuralEditsValid =
    root.querySelectorAll('li[data-id]').length === scenario.size + 1 &&
    insertedNode?.parentElement?.children[insertionIndex] === insertedNode &&
    insertedNode?.querySelector('.name')?.textContent === 'Inserted';
  let deepReorderMs: number | null = null;
  if (scenario.shape === 'deep') {
    siblings.reverse();
    deepReorderMs = measure(() => {
      element.$update();
      element.$flushUpdates();
    });
    structuralEditsValid &&=
      insertedNode?.parentElement?.lastElementChild === insertedNode &&
      root.querySelector('li[data-id="inserted"]') === insertedNode;
  }
  siblings.splice(siblings.indexOf(inserted), 1);
  element.$update();
  element.$flushUpdates();
  structuralEditsValid &&= root.querySelectorAll('li[data-id]').length === scenario.size;
  const currentNodes = new Map(
    Array.from(root.querySelectorAll<HTMLLIElement>('li[data-id]'), node => [node.dataset.id, node]),
  );
  const retainedIdentity = originalNodes.every(node => currentNodes.get(node.dataset.id) === node);
  originalNodes.length = 0;
  currentNodes.clear();

  const removeMs = measure(() => {
    element.items = [];
    element.$flushUpdates();
  });
  const removedItems = root.querySelectorAll('li[data-id]').length;
  const reinsertMs = measure(() => {
    element.items = replacement;
    element.$flushUpdates();
  });
  const reinsertedItems = root.querySelectorAll('li[data-id]').length;
  const reinsertedMarkup = root.innerHTML;
  const markupParity = reinsertedMarkup === parityReference;
  const markupParityDeltaBytes =
    encoder.encode(reinsertedMarkup).length - encoder.encode(parityReference).length;
  const hydrations = element.hydrations;

  // Teardown is scheduled on a microtask by disconnectedCallback, so the
  // synchronous removal cost and the settled cost are reported separately and
  // completion is verified rather than assumed.
  const internals = element as unknown as { $ready: boolean; $root: unknown };
  const disconnectStart = performance.now();
  element.remove();
  const disconnectSyncMs = performance.now() - disconnectStart;
  const settleStart = performance.now();
  await new Promise<void>(resolve => queueMicrotask(resolve));
  await new Promise<void>(resolve => setTimeout(resolve, 0));
  const disconnectSettleMs = performance.now() - settleStart;
  const teardownSettled =
    !element.isConnected && internals.$ready === false && internals.$root === null;
  return {
    mountMs,
    leafUpdateMs,
    ownerUpdateMs,
    replaceRootMs,
    noOpMs,
    reorderMs,
    insertionMs,
    deepReorderMs,
    structuralEditsValid,
    removeMs,
    reinsertMs,
    disconnectSyncMs,
    disconnectSettleMs,
    observedHeapDeltaBytes: beforeHeap === null || observedHeap === null
      ? null
      : observedHeap - beforeHeap,
    htmlBytes,
    mountedItems,
    removedItems,
    reinsertedItems,
    retainedIdentity,
    leafUpdated,
    ownerUpdated,
    noOpMutations,
    hydrations,
    markupParity,
    markupParityDeltaBytes,
    teardownSettled,
  };
}
