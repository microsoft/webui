// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export function inventoryEpoch(): number {
  return window.__webui?.inventoryEpoch ?? 0;
}

export function clearInventory(): void {
  const runtime = window.__webui ??= {};
  runtime.inventory = '';
  runtime.inventoryEpoch = (runtime.inventoryEpoch ?? 0) + 1;
}

/** Merge same-epoch responses; an old request cannot resurrect an evicted bit. */
export function acceptInventory(inventory: string, epoch: number): void {
  if (epoch !== inventoryEpoch()) return;
  const runtime = window.__webui ??= {};
  runtime.inventory = mergeInventory(runtime.inventory ?? '', inventory);
}

export function acceptStyleInventory(inventory: string, epoch: number): void {
  if (epoch !== inventoryEpoch()) return;
  const runtime = window.__webui ??= {};
  runtime.styleInventory = mergeInventory(runtime.styleInventory ?? '', inventory);
}

function mergeInventory(current: string, inventory: string): string {
  const length = Math.max(current.length, inventory.length);
  let merged = '';
  for (let i = 0; i < length; i++) {
    const left = i < current.length ? hexDigit(current.charCodeAt(i)) : 0;
    const right = i < inventory.length ? hexDigit(inventory.charCodeAt(i)) : 0;
    merged += (left | right).toString(16);
  }
  return merged;
}

function hexDigit(code: number): number {
  if (code >= 48 && code <= 57) return code - 48;
  if (code >= 97 && code <= 102) return code - 87;
  if (code >= 65 && code <= 70) return code - 55;
  throw new Error('[WebUI] Invalid component inventory. Use the hexadecimal inventory from the matching protocol.');
}
