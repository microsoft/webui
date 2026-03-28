// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Component inventory bitmask helpers.
 *
 * The inventory is a 256-bit mask (32 bytes, hex-encoded) that tracks which
 * component templates the client has already loaded.  The server checks this
 * via the `X-WebUI-Inventory` header to avoid re-sending known templates.
 *
 * Bit positions are derived from an FNV-1a hash of the component tag name.
 * This implementation **must** stay in sync with the Rust version in
 * `crates/webui-handler/src/route_handler.rs`.
 */

/** FNV-1a hash mod 256 — deterministic bit position for a component name. */
function componentBitPosition(name: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < name.length; i++) {
    hash ^= name.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash % 256;
}

export function parseInventoryHex(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length >> 1);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.substring(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

export function encodeInventoryHex(inv: Uint8Array): string {
  let hex = '';
  for (const b of inv) {
    hex += (b < 16 ? '0' : '') + b.toString(16);
  }
  return hex;
}

export function clearInventoryBit(inv: Uint8Array, name: string): void {
  const bit = componentBitPosition(name);
  const byteIdx = bit >> 3;
  if (byteIdx < inv.length) {
    inv[byteIdx] &= ~(1 << (bit & 7));
  }
}
