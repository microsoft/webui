// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';
import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { acceptInventory, clearInventory, inventoryEpoch } from './inventory.js';

test('concurrent resource responses merge rather than replace unrelated bits', () => {
  window.__webui = { inventory: '01' };
  const epoch = inventoryEpoch();
  acceptInventory('05', epoch);
  acceptInventory('09', epoch);
  assert.equal(window.__webui.inventory, '0d');
  acceptInventory('8001', epoch);
  assert.equal(window.__webui.inventory, '8d01');
});

test('an old resource response cannot resurrect inventory after cache collection', () => {
  window.__webui = { inventory: 'ff' };
  const epoch = inventoryEpoch();
  clearInventory();
  assert.equal(window.__webui.inventory, '');
  assert.equal(inventoryEpoch(), epoch + 1);
  acceptInventory('ff', epoch);
  assert.equal(window.__webui.inventory, '');
  acceptInventory('02', inventoryEpoch());
  assert.equal(window.__webui.inventory, '02');
});

test('malformed inventories fail without mutating the committed value', () => {
  window.__webui = { inventory: '01' };
  assert.throws(() => acceptInventory('0z', inventoryEpoch()), /Invalid component inventory/);
  assert.equal(window.__webui.inventory, '01');
});
