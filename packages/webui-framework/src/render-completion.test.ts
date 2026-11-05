// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  associateRenderTask,
  blockRenderWork,
  cancelRenderTask,
  destroyRenderWork,
  enterRenderTask,
  finishRenderTask,
  finishRenderWork,
  invokeRenderCallback,
  inScope,
  leaveRenderTask,
  MOUNT_WORK,
  observingRender,
  queueRenderTask,
  queueRenderWork,
  renderWork,
  startRenderTask,
  unblockRenderWork,
  UPDATE_WORK,
  whenRendered,
} from './render-completion.js';

const enterRenderWork = (element: Element) => renderWork?.enter(element);
const leaveRenderWork = (element: Element, previous: ReturnType<typeof enterRenderWork>) =>
  renderWork?.leave(element, previous);
const failRenderWork = (element: Element, error: unknown) => renderWork?.fail(element, error);
const disconnectRenderWork = (element: Element) => renderWork?.disconnect(element);
const expectRenderRemoval = (node: Node, end?: Node) => renderWork?.remove(node, end);
const finishRenderRemoval = (previous: readonly Node[] | undefined) => renderWork?.restoreRemoval(previous);

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
  override contains(other: Node | null): boolean { return other === this; }
}

function finish(host: Host, kind = UPDATE_WORK): void {
  const previous = enterRenderWork(host);
  finishRenderWork(host, kind);
  leaveRenderWork(host, previous);
}

test('a native render-root containment proof avoids repeated parent reads', () => {
  const root = new Host();
  class Child extends Host {
    override get parentNode(): ParentNode {
      throw new Error('an already proven lineage should not be walked again');
    }
  }
  const child = new Child();
  let probes = 0;
  const renderRoot = {
    contains(node: Node | null): boolean {
      probes++;
      return node === child;
    },
  };
  Object.defineProperty(root, 'shadowRoot', { value: renderRoot });
  assert.equal(inScope(child, root), true);
  assert.equal(probes, 1);
});

async function collectAfterTurn(): Promise<void> {
  const { setFlagsFromString } = await import('node:v8');
  const { runInNewContext } = await import('node:vm');
  setFlagsFromString('--expose-gc');
  const gc = runInNewContext('gc') as () => void;
  await new Promise(resolve => setTimeout(resolve, 0));
  gc();
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

test('deep synchronous reentrancy restores execution before settling overlapping scopes', async () => {
  const root = new Host();
  const hosts = Array.from({ length: 1024 }, () => {
    const host = new Host();
    host.parent = root;
    return host;
  });
  queueRenderWork(root, UPDATE_WORK);
  const outer = whenRendered(root);
  const previous = hosts.map(host => enterRenderWork(host));
  const inner = whenRendered(hosts[0]);
  for (let i = hosts.length - 1; i >= 0; i--) {
    finishRenderWork(hosts[i], UPDATE_WORK);
    leaveRenderWork(hosts[i], previous[i]);
  }
  await inner;
  finish(root);
  await outer;
  assert.equal(observingRender, false);
  await whenRendered(root);
});

test('nested work remains active after the first boundary fails', async () => {
  const root = new Host();
  queueRenderWork(root, UPDATE_WORK);
  const first = whenRendered(root);
  const error = new Error('render failed');
  const rejected = assert.rejects(first, value => value === error);
  await Promise.resolve();
  const outer = enterRenderWork(root);
  failRenderWork(root, error);
  const inner = enterRenderWork(root);
  let completed = false;
  const second = whenRendered(root).then(() => { completed = true; });
  finishRenderWork(root, UPDATE_WORK);
  leaveRenderWork(root, inner);
  await rejected;
  await Promise.resolve();
  assert.equal(completed, false);
  leaveRenderWork(root, outer);
  await second;
});

test('destroying executing work does not retain its replacement generation', async () => {
  const root = new Host();
  queueRenderWork(root, UPDATE_WORK);
  const first = whenRendered(root);
  const rejected = assert.rejects(first, { name: 'AbortError' });
  await Promise.resolve();
  const outer = enterRenderWork(root);
  destroyRenderWork(root);
  queueRenderWork(root, UPDATE_WORK);
  const second = whenRendered(root);
  const inner = enterRenderWork(root);
  finishRenderWork(root, UPDATE_WORK);
  leaveRenderWork(root, inner);
  await rejected;
  await second;
  leaveRenderWork(root, outer);
});

test('destruction during scope validation does not revive retired work', async () => {
  const root = new Host();
  class RemovedOnCheck extends Host {
    private removeOnRead = true;
    override get isConnected(): boolean {
      if (this.removeOnRead) {
        this.removeOnRead = false;
        destroyRenderWork(this);
      }
      return true;
    }
  }
  const child = new RemovedOnCheck();
  child.parent = root;
  queueRenderWork(child, UPDATE_WORK);
  const rejected = assert.rejects(whenRendered(root), { name: 'AbortError' });
  const previous = enterRenderWork(child);
  leaveRenderWork(child, previous);
  await rejected;
  const next = whenRendered(root);
  const settled = await Promise.race([
    next.then(() => true, () => false),
    new Promise<boolean>(resolve => { setTimeout(() => { resolve(false); }, 0); }),
  ]);
  if (!settled) destroyRenderWork(child);
  assert.equal(settled, true);
});

test('a throwing scope check releases its execution state', async () => {
  const root = new Host();
  const error = new Error('scope check failed');
  class ThrowsOnCheck extends Host {
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
  queueRenderWork(child, UPDATE_WORK);
  let outcome: unknown;
  const first = whenRendered(root);
  void first.then(() => { outcome = 'resolved'; }, value => { outcome = value; });
  try {
    assert.throws(() => enterRenderWork(child), value => value === error);
    await Promise.resolve();
    assert.equal(outcome, error);
    finishRenderWork(child, UPDATE_WORK);
    queueRenderWork(child, UPDATE_WORK);
    const next = whenRendered(root);
    finish(child);
    const settled = await Promise.race([
      next.then(() => true, () => false),
      new Promise<boolean>(resolve => { setTimeout(() => { resolve(false); }, 0); }),
    ]);
    assert.equal(settled, true);
  } finally {
    disconnectRenderWork(root);
    destroyRenderWork(child);
  }
});

test('a throwing discovery releases the completion observer', async () => {
  const root = new Host();
  const error = new Error('discovery failed');
  class ThrowsOnParent extends Host {
    private throwOnRead = true;
    override get parentNode(): ParentNode | null {
      if (this.throwOnRead) {
        this.throwOnRead = false;
        throw error;
      }
      return this.parent;
    }
  }
  const child = new ThrowsOnParent();
  child.parent = root;
  queueRenderWork(child, UPDATE_WORK);
  try {
    await assert.rejects(whenRendered(root), value => value === error);
    assert.equal(observingRender, false);
  } finally {
    disconnectRenderWork(root);
    destroyRenderWork(child);
  }
});

test('completed work leaves no per-element tracking properties', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  const symbols = Object.getOwnPropertySymbols(child);
  queueRenderWork(child, UPDATE_WORK);
  const waiting = whenRendered(root);
  const previous = enterRenderWork(child);
  finishRenderWork(child, UPDATE_WORK);
  leaveRenderWork(child, previous);
  await waiting;
  assert.deepEqual(Object.getOwnPropertySymbols(child), symbols);
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

test('independent lazy work on one descendant is tracked per activation', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  const first = startRenderTask(child);
  const second = startRenderTask(child);
  let completed = false;
  const waiting = whenRendered(root).then(() => { completed = true; });
  finishRenderTask(first);
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(completed, false);
  finishRenderTask(second);
  await waiting;
});

test('retiring one host task group does not capture its later independent work', async () => {
  const root = new Host();
  const firstHost = new Host();
  const secondHost = new Host();
  firstHost.parent = root;
  secondHost.parent = root;
  const first = startRenderTask(firstHost);
  const other = startRenderTask(secondHost);
  const sibling = startRenderTask(firstHost);
  const waiting = whenRendered(root);
  await Promise.resolve();
  finishRenderTask(first);
  finishRenderTask(sibling);
  queueRenderTask(first);
  finishRenderTask(other);
  await waiting;

  let completed = false;
  const later = whenRendered(root).then(() => { completed = true; });
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(completed, false);
  finishRenderTask(first);
  await later;
});

test('retired task groups do not retain their hosts', async () => {
  function retiredHost(): WeakRef<Host> {
    const host = new Host();
    const first = startRenderTask(host);
    const second = startRenderTask(host);
    finishRenderTask(first);
    finishRenderTask(second);
    return new WeakRef(host);
  }
  const host = retiredHost();
  await collectAfterTurn();
  assert.equal(host.deref(), undefined);
});

test('ordinary completion allocates only its collectible membership index', async () => {
  await collectAfterTurn();
  const original = globalThis.WeakMap;
  const indexes: WeakRef<object>[] = [];
  const root = new Host();
  try {
    globalThis.WeakMap = new Proxy(original, {
      construct(target, args) {
        const index = Reflect.construct(target, args) as WeakMap<object, unknown>;
        indexes.push(new WeakRef(index));
        return index;
      },
    });
    queueRenderWork(root, UPDATE_WORK);
    assert.equal(indexes.length, 0, 'ordinary work needs no observation indexes');
    const waiting = whenRendered(root);
    assert.equal(indexes.length, 1, 'execution and absent expected removals need no per-element index');
    finish(root);
    await waiting;
  } finally {
    globalThis.WeakMap = original;
  }
  await collectAfterTurn();
  for (const index of indexes) assert.equal(index.deref(), undefined);
  queueRenderWork(root, UPDATE_WORK);
  const next = whenRendered(root);
  finish(root);
  await next;
  assert.equal(observingRender, false);
});

test('collecting idle observers preserves independently pending mounts and lazy work', async () => {
  const unrelated = new Host();
  const root = new Host();
  queueRenderWork(root, MOUNT_WORK);
  const task = startRenderTask(root);
  await whenRendered(unrelated);
  await collectAfterTurn();
  let completed = false;
  const waiting = whenRendered(root).then(() => { completed = true; });
  finish(root, MOUNT_WORK);
  await Promise.resolve();
  assert.equal(completed, false);
  finishRenderTask(task);
  await waiting;
  assert.equal(completed, true);
});

test('a completed task can be queued again for a later boundary', async () => {
  const root = new Host();
  const task = startRenderTask(root);
  const first = whenRendered(root);
  finishRenderTask(task);
  await first;
  queueRenderTask(task);
  let completed = false;
  const second = whenRendered(root).then(() => { completed = true; });
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(completed, false);
  finishRenderTask(task);
  await second;
});

test('a mount and a lazy activation on the same host complete independently', async () => {
  const root = new Host();
  queueRenderWork(root, MOUNT_WORK);
  const task = startRenderTask(root);
  let completed = false;
  const waiting = whenRendered(root).then(() => { completed = true; });
  finishRenderTask(task);
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(completed, false);
  finish(root, MOUNT_WORK);
  await waiting;
});

test('a later independent activation does not extend an existing completion', async () => {
  const root = new Host();
  const first = startRenderTask(root);
  const waiting = whenRendered(root);
  await Promise.resolve();
  const later = startRenderTask(root);
  finishRenderTask(first);
  await waiting;
  finishRenderTask(later);
});

test('a captured activation carries its boundary to nested asynchronous work', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  const first = startRenderTask(root);
  let completed = false;
  const waiting = whenRendered(root).then(() => { completed = true; });
  await Promise.resolve();
  const previous = enterRenderTask(first);
  const nested = startRenderTask(child);
  leaveRenderTask(previous);
  finishRenderTask(first);
  await Promise.resolve();
  assert.equal(completed, false);
  finishRenderTask(nested);
  await waiting;
});

test('a lazy task can associate an asynchronously mounted descendant', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  const task = startRenderTask(root);
  let completed = false;
  const waiting = whenRendered(root).then(() => { completed = true; });
  await Promise.resolve();
  assert.equal(associateRenderTask(task, child), true);
  const previous = enterRenderWork(child);
  queueRenderWork(child, MOUNT_WORK);
  leaveRenderWork(child, previous);
  finishRenderTask(task);
  await Promise.resolve();
  assert.equal(completed, false);
  finish(child, MOUNT_WORK);
  await waiting;
});

test('lazy work defers readiness errors only until that work finishes', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  const first = startRenderTask(root);
  blockRenderWork(child, 'the component definition has not loaded');
  const waiting = whenRendered(root);
  unblockRenderWork(child);
  finishRenderTask(first);
  await waiting;

  const second = startRenderTask(root);
  blockRenderWork(child, 'the component definition has not loaded');
  const rejected = assert.rejects(whenRendered(root), { name: 'InvalidStateError' });
  finishRenderTask(second);
  await rejected;
  unblockRenderWork(child);
});

test('lazy task failures reject overlapping boundaries with their original error', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  const task = startRenderTask(child);
  const first = whenRendered(root);
  const second = whenRendered(child);
  const error = new Error('lazy load failed');
  const rejectedFirst = assert.rejects(first, value => value === error);
  const rejectedSecond = assert.rejects(second, value => value === error);
  finishRenderTask(task, { value: error });
  await Promise.all([rejectedFirst, rejectedSecond]);
});

test('expected removal retires lazy work and a late old task cannot finish its replacement', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  queueRenderWork(root, UPDATE_WORK);
  const old = startRenderTask(child);
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

  child.connected = true;
  const replacement = startRenderTask(child);
  let completed = false;
  const next = whenRendered(root).then(() => { completed = true; });
  finishRenderTask(old);
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(completed, false);
  finishRenderTask(replacement);
  await next;
});

test('expected removal preserves other work sharing its completion observer', async () => {
  const root = new Host();
  const removed = new Host();
  const retained = new Host();
  const descendant = new Host();
  removed.parent = root;
  retained.parent = root;
  descendant.parent = retained;
  queueRenderWork(root, UPDATE_WORK);
  queueRenderWork(retained, UPDATE_WORK);
  const old = startRenderTask(removed);
  const active = startRenderTask(retained);
  const waiting = whenRendered(root);
  await Promise.resolve();
  const previous = enterRenderWork(root);
  const previousRemoval = expectRenderRemoval(removed);
  removed.connected = false;
  disconnectRenderWork(removed);
  destroyRenderWork(removed);
  finishRenderRemoval(previousRemoval);
  assert.equal(associateRenderTask(active, descendant), true);
  finishRenderWork(root, UPDATE_WORK);
  leaveRenderWork(root, previous);
  finish(retained);
  finish(descendant);
  let completed = false;
  void waiting.then(() => { completed = true; });
  await Promise.resolve();
  assert.equal(completed, false);
  finishRenderTask(active);
  finishRenderTask(old);
  await waiting;
});

test('expected removal retires only the boundary that caused it', async () => {
  const root = new Host();
  const inner = new Host();
  const removed = new Host();
  inner.parent = root;
  removed.parent = inner;
  queueRenderWork(root, UPDATE_WORK);
  const task = startRenderTask(removed);
  const outer = whenRendered(root);
  const innerRejected = assert.rejects(whenRendered(inner), { name: 'AbortError' });
  await Promise.resolve();
  const previous = enterRenderWork(root);
  const previousRemoval = expectRenderRemoval(removed);
  removed.connected = false;
  disconnectRenderWork(removed);
  destroyRenderWork(removed);
  finishRenderRemoval(previousRemoval);
  finishRenderWork(root, UPDATE_WORK);
  leaveRenderWork(root, previous);
  await Promise.all([outer, innerRejected]);
  finishRenderTask(task);
});

test('unexpected lazy task removal or movement rejects the captured completion', async () => {
  const root = new Host();
  const child = new Host();
  child.parent = root;
  const first = startRenderTask(child);
  const removed = assert.rejects(whenRendered(root), { name: 'AbortError' });
  child.connected = false;
  disconnectRenderWork(child);
  destroyRenderWork(child);
  await removed;
  finishRenderTask(first);

  child.connected = true;
  const second = startRenderTask(child);
  const moved = assert.rejects(whenRendered(root), { name: 'AbortError' });
  child.parent = null;
  const previous = enterRenderTask(second);
  leaveRenderTask(previous);
  finishRenderTask(second);
  await moved;
  cancelRenderTask(second);
});
