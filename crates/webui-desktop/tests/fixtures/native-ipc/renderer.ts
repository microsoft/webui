// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createDesktopTransport, IpcError } from '@microsoft/webui-desktop';
import { connectDesktop } from './generated/ts/ipc';
import type { AppConnection } from './generated/ts/ipc';
import type { Item } from './generated/ts/application';
import { checkAdmissionDelivery } from './admission';
import { checkWindowsResources } from './windows-resources';

interface DocumentConnection {
  connection: AppConnection;
  generation: bigint;
  counts: { labels: number; notifications: number };
}
let previousDocument: DocumentConnection | undefined;

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
function item(size: number, phase = 'save'): Item {
  return { id: 18446744073709551615n, image: Uint8Array.from({ length: size }, (_, i) => (i * 31 + 7) % 256), phase };
}
function validate(value: Item): void {
  assert(value.id === 18446744073709551615n, 'uint64 maximum lost');
  assert([0, 16384, 262144].includes(value.image.length), 'unexpected byte length');
  for (let i = 0; i < value.image.length; i++) assert(value.image[i] === (i * 31 + 7) % 256, `byte mismatch at ${i}`);
}
function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>(r => { resolve = r; });
  return { promise, resolve };
}
async function rejected(operation: () => Promise<unknown>, code: string): Promise<boolean> {
  try { await operation(); } catch (error) {
    assert(error instanceof IpcError && error.code === code, `expected ${code}: ${String(error)}`);
    return true;
  }
  throw new Error(`expected rejection ${code}`);
}

export async function run(restored = false): Promise<void> {
  if (!restored && !location.search && !location.hash) await checkWindowsResources();
  if (!restored && !location.search && !location.hash) await checkAdmissionDelivery();
  const retired = restored ? previousDocument : undefined;
  if (restored) {
    assert(retired, 'restored document lost its previous connection');
    const terminal = await retired.connection.closed;
    assert(['navigated', 'closed'].includes(terminal.code), 'old connection terminal reason');
    await rejected(() => retired.connection.host.sessionGeneration(undefined), terminal.code);
  }
  const retiredCounts = retired ? { ...retired.counts } : undefined;
  const counts = { labels: 0, notifications: 0 };
  let labels = 0;
  let changes = 0;
  let confirmations = 0;
  let secondaryChanges = 0;
  let nextChange = deferred();
  const confirmed = deferred();
  const waiting = deferred();
  const secondaryFirst = deferred();
  const transport = createDesktopTransport();
  const connection = await connectDesktop(transport, {
    renderer: {
      async labelFor(value) {
        validate(value);
        await Promise.resolve();
        labels++;
        counts.labels++;
        return { text: `label:${value.id}:${value.image.length}` };
      },
    },
    onError(error) { console.error('NATIVE_IPC_CALLBACK_FAILURE', error); },
  });
  const generation = (await connection.host.sessionGeneration(undefined)).value;
  if (retired) assert(generation > retired.generation, 'BFCache restore reused the old generation');
  connection.renderer.onChanged(() => { counts.notifications++; });
  previousDocument = { connection, generation, counts };
  const stage = new URLSearchParams(location.search).get('native-stage');
  if (stage) {
    if (stage === 'history-a' || stage === 'history-b') {
      const visit = sessionStorage.getItem('native-ipc-history');
      if (visit === 'back-a' || visit === 'return-b') {
        await verifyHistoryConnection(connection, visit, retired, retiredCounts);
      }
    }
    await runLifecycle(connection, stage);
    return;
  }
  await checkSameDocument(connection);
  const subscription = connection.renderer.onChanged(value => {
    validate(value);
    if (value.phase === 'changed') { changes++; nextChange.resolve(); }
    else if (value.phase === 'confirmed') { confirmations++; confirmed.resolve(); }
    else if (value.phase === 'waiting') waiting.resolve();
    else throw new Error(`unexpected phase ${value.phase}`);
  });
  const secondary = connection.renderer.onChanged(value => {
    if (value.phase === 'changed') { secondaryChanges++; secondaryFirst.resolve(); }
  });
  let voidCompleted = true;
  for (const size of [0, 16384, 262144]) {
    nextChange = deferred();
    const previousLabels = labels;
    const result = await connection.host.save(item(size));
    voidCompleted &&= result === undefined && labels === previousLabels + 1;
    await nextChange.promise;
    if (size === 0) { await secondaryFirst.promise; secondary.close(); }
  }
  // The startup handler cannot finish until Release: acceptance is not completion.
  await connection.host.selected(item(16384, 'selected'));
  const notificationAccepted = confirmations === 0;
  await connection.host.release(undefined);
  await confirmed.promise;
  const invalidRejected = await rejected(() => connection.host.save({ ...item(0), id: -1n }), 'invalid-payload');
  const handlerRejected = await rejected(() => connection.host.save(item(0, 'reject')), 'handler');
  const controller = new AbortController();
  const pending = connection.host.wait(item(0), { signal: controller.signal });
  const cancellationResult = rejected(() => pending, 'cancelled');
  await waiting.promise;
  controller.abort();
  const cancelled = await cancellationResult;
  await connection.host.cancellationObserved(undefined);
  // Recovery crosses the same native boundary after cancellation and handler failure.
  nextChange = deferred();
  await connection.host.save(item(16384));
  await nextChange.promise;
  assert(labels === 4 && changes === 4 && confirmations === 1, 'four-flow totals');
  assert(secondaryChanges === 1 && voidCompleted && notificationAccepted, 'subscription/completion semantics');
  subscription.close();
  await connection.host.finish({
    labels, changes, confirmations, invalidRejected, cancelled,
    unsubscribed: secondaryChanges === 1, voidCompleted, notificationAccepted,
    handlerRejected, visibility: document.visibilityState, userAgent: navigator.userAgent,
  });
  const navigationReady = deferred();
  connection.renderer.onChanged(value => {
    validate(value);
    assert(value.phase === 'navigation', 'navigation probe signal');
    navigationReady.resolve();
  });
  void connection.host.lifecycleHold(item(16384, 'navigation')).catch(error => {
    assert(error instanceof IpcError && ['navigated', 'closed'].includes(error.code), 'navigation pending request error');
  });
  await navigationReady.promise;
  // WebKit supplies nil WKNavigation identities for Navigation API document
  // loads. Keep this path distinct from the location.assign/history tests.
  const navigation = (window as Window & {
    navigation?: { navigate(url: string, options: { history: 'replace' }): unknown };
  }).navigation;
  if (navigation) navigation.navigate('/?native-stage=after-navigation', { history: 'replace' });
  else location.replace('/?native-stage=after-navigation');
}

async function verifyHistoryConnection(
  connection: AppConnection,
  visit: string,
  retired: DocumentConnection | undefined,
  retiredCounts: DocumentConnection['counts'] | undefined,
): Promise<void> {
  const phase = `${retired ? 'persisted' : 'fresh'}-${visit === 'back-a' ? 'back' : 'forward'}`;
  const proof = { value: retired?.generation ?? 0n, phase };
  const received = deferred();
  const subscription = connection.renderer.onChanged(value => {
    validate(value);
    assert(value.phase === 'history-probe', 'new document callback probe');
    received.resolve();
  });
  await connection.host.historyProbe(proof);
  await received.promise;
  subscription.close();
  if (retired) {
    assert(retiredCounts && retired.counts.labels === retiredCounts.labels
      && retired.counts.notifications === retiredCounts.notifications, 'retired renderer callbacks ran after restore');
    const terminal = await retired.connection.closed;
    await rejected(() => retired.connection.host.sessionGeneration(undefined), terminal.code);
  }
  await connection.host.historyVerified(proof);
}

async function runLifecycle(connection: AppConnection, stage: string): Promise<void> {
  if (stage === 'history-a' || stage === 'history-b') {
    await runHistory(connection, stage);
    return;
  }
  if (stage === 'after-close') {
    assert(sessionStorage.getItem('native-ipc-close') === 'verified', 'local close assertions missing');
    sessionStorage.removeItem('native-ipc-close');
    await connection.host.lifecycleCheck(item(0, 'close'));
    history.replaceState(null, '', '/?native-stage=history-a');
    sessionStorage.setItem('native-ipc-history', 'first-b');
    await holdForNavigation(connection, 'history-forward');
    location.assign('/?native-stage=history-b');
    return;
  }
  assert(stage === 'after-navigation', 'unknown fixture stage');
  const confirmed = deferred();
  const changed = deferred();
  const closeReady = deferred();
  connection.renderer.onChanged(value => {
    validate(value);
    if (value.phase === 'confirmed-after-navigation') confirmed.resolve();
    else if (value.phase === 'changed') changed.resolve();
    else if (value.phase === 'close') closeReady.resolve();
    else throw new Error(`unexpected post-navigation notification ${value.phase}`);
  });
  // The original registry's startup definition must exist in this new document.
  await connection.host.selected(item(16384, 'after-navigation'));
  await confirmed.promise;
  await connection.host.lifecycleCheck(item(0, 'navigation'));
  await connection.host.save(item(16384));
  await changed.promise;
  const pendingClosed = rejected(() => connection.host.lifecycleHold(item(16384, 'close')), 'closed');
  await closeReady.promise;
  connection.close();
  connection.close();
  assert((await connection.closed).code === 'closed', 'close reason');
  await pendingClosed;
  await rejected(() => connection.host.save(item(0)), 'closed');
  // One native read of the typed session captured by LifecycleHold. No retry,
  // polling, sleep, or application RPC is possible on the now-closed connection.
  const observed = await fetch('/fixture-disconnect-observation', { cache: 'no-store' });
  assert(observed.ok && await observed.text() === 'closed', 'native disconnect not settled before navigation');
  sessionStorage.setItem('native-ipc-close', 'verified');
  location.replace('/?native-stage=after-close');
}

async function checkSameDocument(connection: AppConnection): Promise<void> {
  const original = location.href;
  const generation = await connection.host.sessionGeneration(undefined);
  const hashChanged = new Promise<void>(resolve => {
    window.addEventListener('hashchange', () => resolve(), { once: true });
  });
  location.hash = 'native-same-document';
  await hashChanged;
  await connection.host.sameDocument({ value: generation.value, phase: 'hash' });
  history.pushState({ fixture: true }, '', '/?spa-route=one#native-same-document');
  await connection.host.sameDocument({ value: generation.value, phase: 'spa' });
  const back = new Promise<void>(resolve => {
    window.addEventListener('popstate', () => resolve(), { once: true });
  });
  history.back();
  await back;
  await connection.host.sameDocument({ value: generation.value, phase: 'spa-back' });
  const forward = new Promise<void>(resolve => {
    window.addEventListener('popstate', () => resolve(), { once: true });
  });
  history.forward();
  await forward;
  await connection.host.sameDocument({ value: generation.value, phase: 'spa-forward' });
  history.replaceState(null, '', original);
}

async function holdForNavigation(connection: AppConnection, phase: string): Promise<void> {
  const ready = deferred();
  connection.renderer.onChanged(value => {
    validate(value);
    assert(value.phase === phase, 'history probe phase');
    ready.resolve();
  });
  void connection.host.lifecycleHold(item(16384, phase)).catch(error => {
    assert(error instanceof IpcError && ['navigated', 'closed'].includes(error.code),
      `history pending request error phase=${phase} code=${error?.code} name=${error?.name} message=${error?.message}`);
  });
  await ready.promise;
}

async function runHistory(connection: AppConnection, stage: string): Promise<void> {
  const visit = sessionStorage.getItem('native-ipc-history');
  if (stage === 'history-b' && visit === 'first-b') {
    await connection.host.lifecycleCheck(item(0, 'history-forward'));
    sessionStorage.setItem('native-ipc-history', 'back-a');
    await holdForNavigation(connection, 'history-back');
    history.back();
  } else if (stage === 'history-a' && visit === 'back-a') {
    await connection.host.lifecycleCheck(item(0, 'history-back'));
    sessionStorage.setItem('native-ipc-history', 'return-b');
    await holdForNavigation(connection, 'history-return');
    history.forward();
  } else {
    assert(stage === 'history-b' && visit === 'return-b', 'history traversal state');
    await connection.host.lifecycleCheck(item(0, 'history-return'));
    sessionStorage.removeItem('native-ipc-history');
    // Native shutdown can revoke the document before event acceptance arrives.
    void connection.host.done(undefined).catch(error => {
      assert(error instanceof IpcError && error.code === 'closed', 'unexpected final event error');
    });
  }
}
