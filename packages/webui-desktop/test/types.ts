// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { Connection, NativeIpcBootstrap } from '../src/index.js';
import { save, selected, label, changed, item } from './helpers.js';

function compileOnly(connection: Connection): void {
  const result: Promise<void> = connection.call(save, item);
  void result;
  // @ts-expect-error uint64 must be bigint
  connection.call(save, { ...item, id: 1 });
  // @ts-expect-error renderer methods cannot be called by the host client
  connection.call(label, item);
  // @ts-expect-error notifications are not RPCs
  connection.call(selected, item);
  // @ts-expect-error RPCs are not notifications
  connection.notify(save, item);
  // @ts-expect-error host events cannot be subscribed in renderer
  connection.subscribe(selected, () => {});
  // @ts-expect-error renderer response must match Item
  connection.handle(label, () => 'wrong');
  connection.subscribe(changed, value => { const id: bigint = value.id; void id; });
}
void compileOnly;

function nativeCompileOnly(bootstrap: NativeIpcBootstrap): void {
  bootstrap.disconnect('1', 'a'.repeat(32));
  // @ts-expect-error a generation alone is not disconnect authority
  bootstrap.disconnect('1');
}
void nativeCompileOnly;
