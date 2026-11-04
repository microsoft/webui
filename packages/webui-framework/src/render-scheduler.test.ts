// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  enterRenderWork,
  finishRenderWork,
  leaveRenderWork,
  MOUNT_WORK,
  queueRenderWork,
  whenRendered,
} from './render-completion.js';
import {
  enqueueRender,
  finishRender,
  flushRender,
  pendingRenders,
  RENDER,
  RENDER_PENDING,
} from './render-scheduler.js';

Object.defineProperty(globalThis, 'HTMLElement', {
  value: class {},
  configurable: true,
});

class Target extends HTMLElement {
  parent: Target | null = null;
  override get parentNode(): Target | null { return this.parent; }
  override get isConnected(): boolean { return true; }
  [RENDER_PENDING] = 0;
  run = (): void => {};
  [RENDER](): void {
    try { this.run(); } finally { finishRender(this); }
  }
}

test('one microtask drains a thousand independently queued components', () => {
  withQueue(queued => {
    let rendered = 0;
    for (let i = 0; i < 1000; i++) {
      const target = new Target();
      target.run = () => { rendered++; };
      enqueueRender(target);
    }
    assert.equal(queued.length, 1);
    queued.shift()?.();
    assert.equal(rendered, 1000);
  });
});

function withQueue(check: (queued: VoidFunction[]) => void): void {
  const original = globalThis.queueMicrotask;
  const queued: VoidFunction[] = [];
  globalThis.queueMicrotask = callback => { queued.push(callback); };
  try {
    check(queued);
    assert.equal(pendingRenders.length, 0);
    assert.equal(queued.length, 0);
  } finally {
    while (queued.length) queued.shift()?.();
    globalThis.queueMicrotask = original;
  }
}

test('one drain coalesces writes and processes cascading jobs in FIFO order', () => {
  withQueue(queued => {
    const root = new Target();
    const sibling = new Target();
    const child = new Target();
    const order: string[] = [];
    root.run = () => {
      order.push('root');
      enqueueRender(child);
    };
    sibling.run = () => { order.push('sibling'); };
    child.run = () => { order.push('child'); };
    for (let i = 0; i < 1000; i++) enqueueRender(root);
    enqueueRender(sibling);
    assert.equal(queued.length, 1);
    queued.shift()?.();
    assert.deepEqual(order, ['root', 'sibling', 'child']);
    assert.equal(root[RENDER_PENDING], 0);
    assert.equal(child[RENDER_PENDING], 0);
  });
});

test('a synchronous host flush cancels its stale slot before a new generation', () => {
  withQueue(queued => {
    const first = new Target();
    const second = new Target();
    const order: string[] = [];
    first.run = () => { order.push('first'); };
    second.run = () => { order.push('second'); };
    enqueueRender(first);
    enqueueRender(second);
    flushRender(first);
    enqueueRender(first);
    assert.equal(queued.length, 1);
    queued.shift()?.();
    assert.deepEqual(order, ['first', 'second', 'first']);
  });
});

test('a failed job reports its error and schedules every remaining job', () => {
  withQueue(queued => {
    const failed = new Target();
    const sibling = new Target();
    const child = new Target();
    const error = new Error('render failed');
    const order: string[] = [];
    failed.run = () => {
      enqueueRender(child);
      throw error;
    };
    sibling.run = () => { order.push('sibling'); };
    child.run = () => { order.push('child'); };
    enqueueRender(failed);
    enqueueRender(sibling);
    assert.throws(() => queued.shift()?.(), value => value === error);
    assert.equal(queued.length, 1);
    queued.shift()?.();
    assert.deepEqual(order, ['sibling', 'child']);
  });
});

test('the same drain finishes consequences before later application microtasks', () => {
  withQueue(queued => {
    const root = new Target();
    const child = new Target();
    const order: string[] = [];
    root.run = () => {
      order.push('root');
      enqueueRender(child);
    };
    child.run = () => { order.push('child'); };
    enqueueRender(root);
    queueMicrotask(() => { order.push('application'); });
    queued.shift()?.();
    assert.deepEqual(order, ['root', 'child']);
    queued.shift()?.();
    assert.deepEqual(order, ['root', 'child', 'application']);
  });
});

test('a captured write enrolls a job already queued independently after sealing', async () => {
  const root = new Target();
  const child = new Target();
  const leaf = new Target();
  child.parent = root;
  leaf.parent = child;
  child.run = () => { queueRenderWork(leaf, MOUNT_WORK); };
  queueRenderWork(root, MOUNT_WORK);
  let completed = false;
  const waiting = whenRendered(root).then(() => { completed = true; });
  await Promise.resolve();
  enqueueRender(child);
  const previous = enterRenderWork(root);
  enqueueRender(child);
  finishRenderWork(root, MOUNT_WORK);
  leaveRenderWork(root, previous);
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(completed, false);
  finishRenderWork(leaf, MOUNT_WORK);
  await waiting;
});
