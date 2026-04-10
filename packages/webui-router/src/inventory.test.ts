// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import {
  clearInventoryBit,
  encodeInventoryHex,
  parseInventoryHex,
} from './inventory.js';

describe('inventory helpers', () => {
  test('parseInventoryHex round-trips with encodeInventoryHex', () => {
    const hex = '2000000000000000000000000000000000000000000000000001000000000000';
    const bytes = parseInventoryHex(hex);
    assert.equal(encodeInventoryHex(bytes), hex);
  });

  test('parseInventoryHex handles empty string', () => {
    const bytes = parseInventoryHex('');
    assert.equal(bytes.length, 0);
    assert.equal(encodeInventoryHex(bytes), '');
  });

  test('clearInventoryBit clears the correct bit for a component', () => {
    // 'section-page' hashes to bit 200 → byte 25, mask 0x01.
    // Start with byte 25 = 0x01 (bit set).
    const inv = new Uint8Array(32);
    inv[25] = 0x01;
    clearInventoryBit(inv, 'section-page');
    assert.equal(inv[25], 0x00, 'bit for section-page should be cleared');
  });

  test('clearInventoryBit does not disturb other bits', () => {
    // 'routes-app' hashes to bit 5 → byte 0, mask 0x20 (0b00100000).
    // Set byte 0 to 0xff so all bits are set.
    const inv = new Uint8Array(32);
    inv[0] = 0xff;
    clearInventoryBit(inv, 'routes-app');
    assert.equal(inv[0], 0xff & ~0x20, 'only bit 5 should be cleared');
  });

  test('clearInventoryBit is safe when byte index is out of range', () => {
    const inv = new Uint8Array(1); // too small for most components
    // Should not throw — just no-ops
    clearInventoryBit(inv, 'section-page');
    assert.equal(inv[0], 0x00);
  });

  // Cross-check: FNV-1a bit positions must match the Rust implementation
  // in crates/webui-handler/src/route_handler.rs.
  test('FNV-1a bit positions match server (cross-check)', () => {
    // Known vectors computed from the Rust implementation:
    //   section-page → bit 200 (byte 25, mask 0x01)
    //   topic-page   → bit 192 (byte 24, mask 0x01)
    //   routes-app   → bit 5   (byte 0,  mask 0x20)
    //   user-detail  → bit 252 (byte 31, mask 0x10)
    //   home-page    → bit 186 (byte 23, mask 0x04)
    const cases: Array<[string, number, number]> = [
      ['section-page', 25, 0x01],
      ['topic-page', 24, 0x01],
      ['routes-app', 0, 0x20],
      ['user-detail', 31, 0x10],
      ['home-page', 23, 0x04],
    ];

    for (const [name, byteIdx, mask] of cases) {
      // Set the expected bit, then clear it — verifies the hash lands correctly.
      const inv = new Uint8Array(32);
      inv[byteIdx] = mask;
      clearInventoryBit(inv, name);
      assert.equal(
        inv[byteIdx], 0x00,
        `clearInventoryBit('${name}') should clear byte ${byteIdx} mask 0x${mask.toString(16)}`,
      );
    }
  });
});
