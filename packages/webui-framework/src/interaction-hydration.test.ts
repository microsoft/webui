// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  installInteractionHydration,
  isInteractionReplay,
} from './interaction-hydration.js';

type FakeMouseEventInit = EventInit & Partial<Pick<
  MouseEvent,
  'altKey' | 'button' | 'ctrlKey' | 'metaKey' | 'shiftKey'
>>;

class FakeMouseEvent extends Event {
  readonly altKey: boolean;
  readonly button: number;
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
  readonly shiftKey: boolean;

  constructor(type: string, init: FakeMouseEventInit = {}) {
    super(type, init);
    this.altKey = init.altKey ?? false;
    this.button = init.button ?? 0;
    this.ctrlKey = init.ctrlKey ?? false;
    this.metaKey = init.metaKey ?? false;
    this.shiftKey = init.shiftKey ?? false;
  }
}

class FakeElement extends EventTarget {
  appClicks = 0;
  readonly ownerDocument = {
    defaultView: { MouseEvent: FakeMouseEvent },
  };

  click(init: FakeMouseEventInit = {}): FakeMouseEvent {
    const event = new FakeMouseEvent('click', {
      bubbles: true,
      cancelable: true,
      ...init,
    });
    this.dispatchEvent(event);
    return event;
  }
}

function install(
  root: FakeElement,
  load: () => Promise<unknown>,
  onError?: (error: unknown) => void,
): () => void {
  return installInteractionHydration({
    load,
    onError,
    root: root as unknown as Element,
  });
}

async function settle(): Promise<void> {
  await new Promise<void>((resolve) => setImmediate(resolve));
}

test('loads once and replays on the composed-path target', async () => {
  const root = new FakeElement();
  const target = new FakeElement();
  let loads = 0;
  let replayed = false;
  target.addEventListener('click', (event) => {
    target.appClicks++;
    replayed = isInteractionReplay(event);
  });
  install(root, async () => {
    loads++;
  });

  const click = new FakeMouseEvent('click', {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(click, 'composedPath', {
    value: () => [target, root],
  });
  root.dispatchEvent(click);
  await settle();

  assert.equal(click.defaultPrevented, true);
  assert.equal(loads, 1);
  assert.equal(target.appClicks, 1);
  assert.equal(replayed, true);
});

test('wake signals share one load without hover or cancellation', async () => {
  const root = new FakeElement();
  let release!: () => void;
  let loads = 0;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  install(root, () => {
    loads++;
    return pending;
  });

  root.dispatchEvent(new Event('pointerover'));
  await settle();
  assert.equal(loads, 0);

  for (const type of ['pointerdown', 'focusin', 'keydown']) {
    const event = new Event(type, { bubbles: true, cancelable: true });
    root.dispatchEvent(event);
    assert.equal(event.defaultPrevented, false);
  }
  const click = root.click();
  assert.equal(click.defaultPrevented, true);
  release();
  await pending;
  await settle();
  assert.equal(loads, 1);
});

test('ineligible clicks pass through while loading', async () => {
  const root = new FakeElement();
  let release!: () => void;
  let loads = 0;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  install(root, () => {
    loads++;
    return pending;
  });
  root.addEventListener('click', () => {
    root.appClicks++;
  });

  const modified = root.click({ ctrlKey: true });
  const cancelled = new FakeMouseEvent('click', { cancelable: true });
  cancelled.preventDefault();
  root.dispatchEvent(cancelled);
  const generic = new Event('click', { cancelable: true });
  root.dispatchEvent(generic);
  release();
  await settle();

  assert.equal(modified.defaultPrevented, false);
  assert.equal(cancelled.defaultPrevented, true);
  assert.equal(generic.defaultPrevented, false);
  assert.equal(loads, 1);
  assert.equal(root.appClicks, 3);
});

test('failure replay bypasses a same-root replacement boundary', async () => {
  const root = new FakeElement();
  const failure = new Error('chunk unavailable');
  let replacementLoads = 0;
  let reported: unknown;
  install(root, async () => {
    throw failure;
  }, (error) => {
    reported = error;
    install(root, async () => {
      replacementLoads++;
    });
  });
  root.addEventListener('click', () => {
    root.appClicks++;
  });

  root.click();
  await settle();

  assert.equal(reported, failure);
  assert.equal(replacementLoads, 0);
  assert.equal(root.appClicks, 1);
});

test('replay still waits for an unvisited nested boundary', async () => {
  const outer = new FakeElement();
  const inner = new FakeElement();
  let innerLoads = 0;
  install(outer, async () => undefined);
  install(inner, async () => {
    innerLoads++;
  });
  inner.addEventListener('click', () => {
    inner.appClicks++;
  });

  const click = new FakeMouseEvent('click', { cancelable: true });
  Object.defineProperty(click, 'composedPath', {
    value: () => [inner, outer],
  });
  outer.dispatchEvent(click);
  await settle();

  assert.equal(innerLoads, 1);
  assert.equal(inner.appClicks, 1);
});

test('disposal, deduplication, and reinstallation share one lifecycle', async () => {
  const root = new FakeElement();
  let loads = 0;
  const first = install(root, async () => {
    loads++;
  });
  assert.equal(install(root, async () => undefined), first);
  first();
  root.click();
  await settle();
  assert.equal(loads, 0);

  install(root, async () => {
    loads++;
  });
  root.click();
  await settle();
  install(root, async () => {
    loads++;
  });
  root.click();
  await settle();
  assert.equal(loads, 2);
});

test('a throwing error callback cannot block fallback replay', async () => {
  const root = new FakeElement();
  const previousConsoleError = console.error;
  let loggedErrors = 0;
  console.error = () => {
    loggedErrors++;
  };
  try {
    install(root, async () => {
      throw new Error('chunk unavailable');
    }, () => {
      throw new Error('reporter unavailable');
    });
    root.addEventListener('click', () => {
      root.appClicks++;
    });
    root.click();
    await settle();

    assert.equal(root.appClicks, 1);
    assert.equal(loggedErrors, 1);
  } finally {
    console.error = previousConsoleError;
  }
});
