// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import { canDelegateEvent } from './events.js';

describe('canDelegateEvent — composed-event whitelist', () => {
  test('hyphenated custom events always delegate (composed by convention)', () => {
    assert.equal(canDelegateEvent('toggle-nav-pane'), true);
    assert.equal(canDelegateEvent('input-change'), true);
    assert.equal(canDelegateEvent('cart-state'), true);
  });

  test('common composed events bubble through shadow DOM and delegate', () => {
    // Mouse / pointer / touch / keyboard / wheel / drag — all `composed: true`.
    for (const name of [
      'click', 'dblclick', 'contextmenu', 'mousedown', 'mouseup', 'mousemove',
      'mouseover', 'mouseout', 'pointerdown', 'pointerup', 'pointermove',
      'pointerover', 'pointerout', 'pointercancel', 'touchstart', 'touchend',
      'touchmove', 'touchcancel', 'keydown', 'keyup', 'keypress', 'wheel',
      'input', 'beforeinput', 'drag', 'dragstart', 'dragend', 'dragenter',
      'dragleave', 'dragover', 'drop',
    ]) {
      assert.equal(canDelegateEvent(name), true, `${name} should delegate`);
    }
  });

  // Regression: commit `de95f66f` mistakenly added these to the whitelist.
  // Per HTML spec they have `bubbles: true` but `composed: false`, so a
  // document-level listener never observes them when they fire inside a
  // component's shadow DOM. Document-delegation broke <form @submit>
  // handlers (commerce add-to-cart) and @change-bound form controls.
  test('non-composed events are NOT delegated (must use direct listeners)', () => {
    assert.equal(
      canDelegateEvent('submit'), false,
      'submit is composed: false — would never reach document from a shadow form',
    );
    assert.equal(
      canDelegateEvent('reset'), false,
      'reset is composed: false — same shadow-boundary problem as submit',
    );
    assert.equal(
      canDelegateEvent('change'), false,
      'change on form controls is composed: false — does not cross shadow root',
    );
  });

  test('non-bubbling events are NOT delegated', () => {
    // focus/blur/scroll/load do not bubble — direct-listener path is required.
    assert.equal(canDelegateEvent('focus'), false);
    assert.equal(canDelegateEvent('blur'), false);
    assert.equal(canDelegateEvent('scroll'), false);
    assert.equal(canDelegateEvent('load'), false);
    assert.equal(canDelegateEvent('mouseenter'), false);
    assert.equal(canDelegateEvent('mouseleave'), false);
    assert.equal(canDelegateEvent('pointerenter'), false);
    assert.equal(canDelegateEvent('pointerleave'), false);
  });

  test('event names are case-insensitive', () => {
    assert.equal(canDelegateEvent('CLICK'), true);
    assert.equal(canDelegateEvent('Click'), true);
    assert.equal(canDelegateEvent('SUBMIT'), false);
  });

  test('unknown event names are NOT delegated by default', () => {
    assert.equal(canDelegateEvent('madeupevent'), false);
    assert.equal(canDelegateEvent(''), false);
  });
});
