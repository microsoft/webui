// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  destroyRenderWork,
  disconnectRenderWork,
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

test('queued membership still rejects a target moved outside the captured scope', async () => {
  const root = new Target();
  const child = new Target();
  child.parent = root;
  enqueueRender(child);
  const waiting = whenRendered(root);
  const rejected = assert.rejects(waiting, { name: 'AbortError' });
  child.parent = null;
  await rejected;
});

test('moving queued work rejects only the boundary it leaves', async () => {
  const root = new Target();
  const elsewhere = new Target();
  const child = new Target();
  const leaf = new Target();
  child.parent = root;
  leaf.parent = child;
  child.run = () => { queueRenderWork(leaf, MOUNT_WORK); };
  enqueueRender(child);
  const outside = assert.rejects(whenRendered(root), { name: 'AbortError' });
  let innerFinished = false;
  const inside = whenRendered(child).then(() => { innerFinished = true; });
  child.parent = elsewhere;
  await outside;
  await Promise.resolve();
  assert.equal(innerFinished, false);
  finishRenderWork(leaf, MOUNT_WORK);
  await inside;
});

test('a failed scope check retries unattempted work and reports the error', async () => {
  const error = new Error('scope check failed');
  let first: Promise<void> | undefined;
  withQueue(queued => {
    const root = new Target();
    class ThrowsOnCheck extends Target {
      private throwOnRead = true;
      override get isConnected(): boolean {
        if (this.throwOnRead) {
          this.throwOnRead = false;
          throw error;
        }
        return true;
      }
    }
    const child = new ThrowsOnCheck();
    child.parent = root;
    let runs = 0;
    child.run = () => { runs++; };
    try {
      enqueueRender(child);
      first = whenRendered(root);
      void first.catch(() => {});
      assert.throws(() => queued.shift()?.(), value => value === error);
      assert.notEqual(child[RENDER_PENDING], 0);
      while (queued.length) queued.shift()?.();
      assert.equal(runs, 1);
      assert.equal(child[RENDER_PENDING], 0);
    } finally {
      disconnectRenderWork(root);
      destroyRenderWork(child);
    }
  });
  assert.ok(first);
  await assert.rejects(first, value => value === error);
});

test('a boundary opened within a render captures subsequent coalesced child work', async () => {
  const root = new Target();
  const child = new Target();
  const leaf = new Target();
  child.parent = root;
  leaf.parent = child;
  let inner: Promise<void> | undefined;
  root.run = () => {
    enqueueRender(child);
    inner = whenRendered(child);
    enqueueRender(child);
  };
  child.run = () => { queueRenderWork(leaf, MOUNT_WORK); };
  enqueueRender(root);
  const outer = whenRendered(root);
  let finished = 0;
  await Promise.resolve();
  void outer.then(() => { finished++; });
  void inner!.then(() => { finished++; });
  await Promise.resolve();
  assert.equal(finished, 0);
  finishRenderWork(leaf, MOUNT_WORK);
  await Promise.all([outer, inner]);
  assert.equal(finished, 2);
});
