// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import { NON_BUBBLING_EVENTS } from './non-bubbling-events.js';

describe('NON_BUBBLING_EVENTS', () => {
  test('lists focus/blur so their bindings are wired directly', () => {
    assert.equal(NON_BUBBLING_EVENTS.has('focus'), true);
    assert.equal(NON_BUBBLING_EVENTS.has('blur'), true);
  });

  test('excludes the bubbling focus counterparts', () => {
    assert.equal(NON_BUBBLING_EVENTS.has('focusin'), false);
    assert.equal(NON_BUBBLING_EVENTS.has('focusout'), false);
  });

  test('lists pointer boundary events but not their bubbling counterparts', () => {
    for (const name of ['mouseenter', 'mouseleave', 'pointerenter', 'pointerleave']) {
      assert.equal(NON_BUBBLING_EVENTS.has(name), true, name);
    }
    for (const name of ['mouseover', 'mouseout', 'pointerover', 'pointerout']) {
      assert.equal(NON_BUBBLING_EVENTS.has(name), false, name);
    }
  });

  test('lists resource, dialog, and media events', () => {
    for (const name of ['load', 'error', 'abort', 'toggle', 'close', 'cancel', 'invalid', 'scroll', 'play', 'ended', 'timeupdate']) {
      assert.equal(NON_BUBBLING_EVENTS.has(name), true, name);
    }
  });

  test('excludes common bubbling events so they keep using delegation', () => {
    for (const name of ['click', 'input', 'change', 'keydown', 'submit', 'pointerdown', 'transitionend', 'animationend', 'select', 'slotchange']) {
      assert.equal(NON_BUBBLING_EVENTS.has(name), false, name);
    }
  });
});
