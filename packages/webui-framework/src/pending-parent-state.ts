// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** State written by a parent before a compiled child upgrades. */
export interface PendingParentState {
  readonly values: Record<string, unknown>;
  replay?: Set<string>;
}

const pendingParentStateByElement = new WeakMap<Element, PendingParentState>();

/** Queue one parent value until an unupgraded compiled child can consume it. */
export function queuePendingParentState(
  element: Element,
  name: string,
  value: unknown,
  replayAfterHydration: boolean,
): void {
  let pending = pendingParentStateByElement.get(element);
  if (!pending) {
    pending = {
      values: Object.create(null) as Record<string, unknown>,
    };
    pendingParentStateByElement.set(element, pending);
  }
  pending.values[name] = value;
  if (replayAfterHydration) {
    (pending.replay ??= new Set()).add(name);
  }
}

/** Take and release all state queued for one compiled child instance. */
export function consumePendingParentState(
  element: Element,
): PendingParentState | undefined {
  const pending = pendingParentStateByElement.get(element);
  if (pending) pendingParentStateByElement.delete(element);
  return pending;
}
