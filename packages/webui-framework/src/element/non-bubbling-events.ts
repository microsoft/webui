// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * DOM events that do **not** bubble when dispatched at an element.
 *
 * Element event bindings (`@click`, `@focus`, …) are normally wired as a single
 * delegated listener on the component render root: one listener serves every
 * binding of that event name in the block, and the handler is matched by
 * walking up from `event.target`.  That only works for events that propagate,
 * so events listed here are wired as direct listeners on the bound element
 * instead — otherwise the binding would silently never fire.
 *
 * Only events that can be dispatched at an element are listed.  Bubbling
 * counterparts are intentionally absent: `focusin`/`focusout` bubble while
 * `focus`/`blur` do not, and `mouseover`/`mouseout` bubble while
 * `mouseenter`/`mouseleave` do not.
 *
 * @see https://developer.mozilla.org/docs/Web/API/Event/bubbles
 */
export const NON_BUBBLING_EVENTS: ReadonlySet<string> = new Set([
  // Focus — focusin/focusout are the bubbling counterparts.
  'focus',
  'blur',

  // Pointer boundary — mouseover/mouseout are the bubbling counterparts.
  'mouseenter',
  'mouseleave',
  'pointerenter',
  'pointerleave',

  // Resource loading (<img>, <script>, <link>, <iframe>, media).
  'load',
  'error',
  'abort',
  'loadstart',
  'loadend',
  'progress',

  // Scrolling and sizing.
  'scroll',
  'scrollend',
  'resize',

  // Form validation, <dialog>, and popover state.
  'invalid',
  'cancel',
  'close',
  'toggle',
  'beforetoggle',

  // Media elements (<audio>, <video>).
  'canplay',
  'canplaythrough',
  'durationchange',
  'emptied',
  'encrypted',
  'ended',
  'loadeddata',
  'loadedmetadata',
  'pause',
  'play',
  'playing',
  'ratechange',
  'seeked',
  'seeking',
  'stalled',
  'suspend',
  'timeupdate',
  'volumechange',
  'waiting',
  'waitingforkey',
]);
