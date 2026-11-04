// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  blockRenderWork,
  destroyRenderWork,
  disconnectRenderWork,
  enterRenderWork,
  expectRenderRemoval,
  failRenderWork,
  finishRenderWork,
  finishRenderRemoval,
  invokeRenderCallback,
  leaveRenderWork,
  MOUNT_WORK,
  queueRenderWork,
  unblockRenderWork,
  UPDATE_WORK,
  whenRendered,
} from './render-completion.js';

Object.defineProperty(globalThis, 'HTMLElement', {
  value: class {},
  configurable: true,
});

class Host extends HTMLElement {
  parent: Host | null = null;
  connected = true;
  override get parentElement(): Host | null { return this.parent; }
  override get parentNode(): ParentNode | null { return this.parent; }
  override get assignedSlot(): HTMLSlotElement | null { return null; }
  override get localName(): string { return 'test-render'; }
  override get isConnected(): boolean { return this.connected; }
  override getRootNode(): Node { return this; }
}

function finish(host: Host, kind = UPDATE_WORK): void {
  const previous = enterRenderWork(host);
  finishRenderWork(host, kind);
  leaveRenderWork(host, previous);
}

test('idle completion is asynchronous and rejects an unavailable root', async () => {
  const root = new Host();
  let complete = false;
  const waiting = whenRendered(root).then(() => { complete = true; });
  assert.equal(complete, false);
  await waiting;
  assert.equal(complete, true);
  await assert.rejects(whenRendered(root, 'hydration is deferred'), { name: 'InvalidStateError' });
});

test('captures already-pending descendants but not unrelated work', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  const unrelated = new Host();
  queueRenderWork(child, UPDATE_WORK);
  queueRenderWork(unrelated, UPDATE_WORK);
  const waiting = whenRendered(root);
  finish(child);
  await waiting;
  destroyRenderWork(unrelated);
});

test('captures descendants through a foreign-realm shadow root', async () => {
  const root = new Host();
  class ForeignShadowRoot extends Host {
    override readonly nodeType = 11;
    readonly host = root;
  }
  const shadow = new ForeignShadowRoot();
  class ForeignShadowChild extends Host {
    override get parentNode(): ParentNode { return shadow; }
    override getRootNode(): Node {
      return shadow;
    }
  }
  const child = new ForeignShadowChild();
  queueRenderWork(child, UPDATE_WORK);
  try {
    let completed = false;
    const waiting = whenRendered(root).then(() => { completed = true; });
    await Promise.resolve();
    await Promise.resolve();
    assert.equal(completed, false);
    finish(child);
    await waiting;
    assert.equal(completed, true);
  } finally {
    destroyRenderWork(child);
  }
});

test('enrollment includes the setter trailing a Changed callback', async () => {
  const root = new Host();
  const waiting = whenRendered(root);
  queueRenderWork(root, UPDATE_WORK);
  await Promise.resolve();
  let complete = false;
  void waiting.then(() => { complete = true; });
  await Promise.resolve();
  assert.equal(complete, false);
  finish(root);
  await waiting;
});

test('captures reentrant descendant work after enrollment closes', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  queueRenderWork(root, UPDATE_WORK);
  const waiting = whenRendered(root);
  await Promise.resolve();
  const previous = enterRenderWork(root);
  queueRenderWork(child, UPDATE_WORK);
  finishRenderWork(root, UPDATE_WORK);
  leaveRenderWork(root, previous);
  let complete = false;
  void waiting.then(() => { complete = true; });
  await Promise.resolve();
  assert.equal(complete, false);
  finish(child);
  await waiting;
});

test('later independent work does not extend the captured batch', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  queueRenderWork(root, UPDATE_WORK);
  const waiting = whenRendered(root);
  await Promise.resolve();
  queueRenderWork(child, UPDATE_WORK);
  finish(root);
  await waiting;
  destroyRenderWork(child);
});

test('a started mount carries its boundary to further descendant updates', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  queueRenderWork(root, MOUNT_WORK);
  const waiting = whenRendered(root);
  await Promise.resolve();
  const previous = enterRenderWork(root);
  queueRenderWork(child, UPDATE_WORK);
  finishRenderWork(root, MOUNT_WORK);
  leaveRenderWork(root, previous);
  finish(child);
  await waiting;
});

test('blocked pre-call work rejects without permanently poisoning readiness', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  blockRenderWork(child, 'the component definition has not loaded');
  await assert.rejects(whenRendered(root), { name: 'InvalidStateError' });
  unblockRenderWork(child);
  await whenRendered(root);
});

test('render failures reject overlapping waiters with the original error', async () => {
  const root = new Host();
  queueRenderWork(root, UPDATE_WORK);
  const first = whenRendered(root);
  const second = whenRendered(root);
  const error = new Error('render failed');
  const firstRejected = assert.rejects(first, value => value === error);
  const secondRejected = assert.rejects(second, value => value === error);
  failRenderWork(root, error);
  finish(root);
  await Promise.all([firstRejected, secondRejected]);
  await whenRendered(root);
});

test('disconnecting the root rejects instead of waiting for reconnect', async () => {
  const root = new Host();
  queueRenderWork(root, UPDATE_WORK);
  const waiting = whenRendered(root);
  const rejected = assert.rejects(waiting, { name: 'AbortError' });
  root.connected = false;
  disconnectRenderWork(root);
  destroyRenderWork(root);
  await rejected;
});

test('expected structural disposal retires captured descendant work', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  queueRenderWork(root, UPDATE_WORK);
  queueRenderWork(child, UPDATE_WORK);
  const waiting = whenRendered(root);
  await Promise.resolve();
  const previous = enterRenderWork(root);
  const previousRemoval = expectRenderRemoval(child);
  child.connected = false;
  disconnectRenderWork(child);
  destroyRenderWork(child);
  finishRenderRemoval(previousRemoval);
  finishRenderWork(root, UPDATE_WORK);
  leaveRenderWork(root, previous);
  await waiting;
});

test('nested execution does not complete work while its outer render still runs', async () => {
  const root = new Host();
  queueRenderWork(root, UPDATE_WORK);
  const waiting = whenRendered(root);
  await Promise.resolve();
  const outer = enterRenderWork(root);
  const inner = enterRenderWork(root);
  finishRenderWork(root, UPDATE_WORK);
  leaveRenderWork(root, inner);
  let complete = false;
  void waiting.then(() => { complete = true; });
  await Promise.resolve();
  assert.equal(complete, false);
  leaveRenderWork(root, outer);
  await waiting;
});

test('blocked staged targets are checked after a captured render inserts them', async () => {
  const root = new Host();
  const child = new Host();
  queueRenderWork(root, UPDATE_WORK);
  const waiting = whenRendered(root);
  const rejected = assert.rejects(waiting, { name: 'InvalidStateError' });
  await Promise.resolve();
  const previous = enterRenderWork(root);
  blockRenderWork(child, 'the component definition has not loaded');
  child.parent = root;
  finishRenderWork(root, UPDATE_WORK);
  leaveRenderWork(root, previous);
  await rejected;
  unblockRenderWork(child);
});

test('unexpected reparenting rejects a captured job instead of reporting success', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  queueRenderWork(child, UPDATE_WORK);
  const waiting = whenRendered(root);
  const rejected = assert.rejects(waiting, { name: 'AbortError' });
  await Promise.resolve();
  child.parent = null;
  finish(child);
  await rejected;
});

test('a later generation is not retained by a settled request', async () => {
  const root = new Host();
  queueRenderWork(root, UPDATE_WORK);
  const first = whenRendered(root);
  finish(root);
  await first;
  queueRenderWork(root, UPDATE_WORK);
  const second = whenRendered(root);
  let complete = false;
  void second.then(() => { complete = true; });
  await Promise.resolve();
  assert.equal(complete, false);
  finish(root);
  await second;
});

test('a callback opening the first boundary still reports its thrown error', async () => {
  const root = new Host();
  const error = new Error('callback failed');
  let waiting: Promise<void> | undefined;
  assert.throws(() => invokeRenderCallback(root, () => {
    waiting = whenRendered(root);
    throw error;
  }, undefined, undefined), error);
  assert.ok(waiting);
  await assert.rejects(waiting, error);
});

test('author removal of a sibling is not mistaken for structural disposal', async () => {
  const root = new Host();
  const removed = new Host();
  const sibling = new Host();
  removed.parent = root;
  sibling.parent = root;
  queueRenderWork(root, UPDATE_WORK);
  queueRenderWork(sibling, UPDATE_WORK);
  const waiting = whenRendered(root);
  const rejected = assert.rejects(waiting, { name: 'AbortError' });
  await Promise.resolve();
  const previous = enterRenderWork(root);
  const previousRemoval = expectRenderRemoval(removed);
  sibling.parent = null;
  sibling.connected = false;
  disconnectRenderWork(sibling);
  destroyRenderWork(sibling);
  finishRenderRemoval(previousRemoval);
  finishRenderWork(root, UPDATE_WORK);
  leaveRenderWork(root, previous);
  await rejected;
});

test('a later independent blocker does not replace a retired staged generation', async () => {
  const root = new Host();
  const child = new Host();
  queueRenderWork(root, MOUNT_WORK | UPDATE_WORK);
  const waiting = whenRendered(root);
  await Promise.resolve();
  const previous = enterRenderWork(root);
  blockRenderWork(child, 'the component definition has not loaded');
  unblockRenderWork(child);
  child.parent = root;
  finishRenderWork(root, UPDATE_WORK);
  leaveRenderWork(root, previous);
  blockRenderWork(child, 'a later independent write is deferred');
  finish(root, MOUNT_WORK);
  try {
    await waiting;
  } finally {
    unblockRenderWork(child);
  }
});

test('finishing a middle pending job preserves discovery of remaining jobs', async () => {
  const root = new Host();
  const children = [new Host(), new Host(), new Host()];
  for (const child of children) {
    child.parent = root;
    queueRenderWork(child, UPDATE_WORK);
  }
  finish(children[1]);
  let completed = false;
  const waiting = whenRendered(root).then(() => { completed = true; });
  finish(children[0]);
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(completed, false);
  finish(children[2]);
  await waiting;
});

test('overlapping failure does not release another boundary sharing its remaining jobs', async () => {
  const root = new Host();
  const failed = new Host();
  const child = new Host();
  const leaf = new Host();
  failed.parent = root;
  child.parent = root;
  leaf.parent = child;
  for (const target of [failed, child, leaf]) queueRenderWork(target, UPDATE_WORK);
  const outer = whenRendered(root);
  let completed = false;
  const inner = whenRendered(child).then(() => { completed = true; });
  const error = new Error('one branch failed');
  const rejected = assert.rejects(outer, value => value === error);
  failRenderWork(failed, error);
  finish(failed);
  finish(child);
  await rejected;
  assert.equal(completed, false);
  finish(leaf);
  await inner;
  await whenRendered(root);
});

test('an update cannot retire a still-pending mount on the same component', async () => {
  const root = new Host();
  queueRenderWork(root, MOUNT_WORK);
  queueRenderWork(root, UPDATE_WORK);
  let completed = false;
  const waiting = whenRendered(root).then(() => { completed = true; });
  finish(root);
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(completed, false);
  finish(root, MOUNT_WORK);
  await waiting;
});
