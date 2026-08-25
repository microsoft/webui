// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  installInteractionHydration,
  isInteractionReplay,
} from './interaction-hydration.js';

class FakeElement extends EventTarget {
  appClicks = 0;
  readonly ownerDocument = {
    defaultView: {
      MouseEvent: FakeMouseEvent,
    },
  };

  click(): void {
    this.dispatchEvent(new FakeMouseEvent('click', {
      bubbles: true,
      cancelable: true,
    }));
  }
}

class FakeMouseEvent extends Event {
  readonly altKey: boolean;
  readonly button = 0;
  readonly buttons = 0;
  readonly clientX = 0;
  readonly clientY = 0;
  readonly ctrlKey: boolean;
  readonly detail = 1;
  readonly metaKey: boolean;
  readonly movementX = 0;
  readonly movementY = 0;
  readonly relatedTarget = null;
  readonly screenX = 0;
  readonly screenY = 0;
  readonly shiftKey: boolean;
  readonly view = null;

  constructor(
    type: string,
    init: EventInit & {
      altKey?: boolean;
      ctrlKey?: boolean;
      metaKey?: boolean;
      shiftKey?: boolean;
    },
  ) {
    super(type, init);
    this.altKey = init.altKey ?? false;
    this.ctrlKey = init.ctrlKey ?? false;
    this.metaKey = init.metaKey ?? false;
    this.shiftKey = init.shiftKey ?? false;
  }
}

async function flushPromises(): Promise<void> {
  await new Promise<void>((resolve) => {
    setImmediate(resolve);
  });
}

test('loads once and replays the first click after hydration', async () => {
  const root = new FakeElement() as unknown as Element;
  let loads = 0;
  let replayed = false;
  installInteractionHydration({
    load: async () => {
      loads++;
    },
    root,
  });
  root.addEventListener('click', (event) => {
    (root as unknown as FakeElement).appClicks++;
    replayed = isInteractionReplay(event);
  });

  (root as unknown as FakeElement).click();
  await flushPromises();

  assert.equal(loads, 1);
  assert.equal((root as unknown as FakeElement).appClicks, 1);
  assert.equal(replayed, true);
});

test('pointer down preload and click share one import without hover work', async () => {
  const root = new FakeElement() as unknown as Element;
  let resolveLoad!: () => void;
  let loads = 0;
  const pending = new Promise<void>((resolve) => {
    resolveLoad = resolve;
  });
  installInteractionHydration({
    load: () => {
      loads++;
      return pending;
    },
    root,
  });

  root.dispatchEvent(new Event('pointerover'));
  await flushPromises();
  assert.equal(loads, 0);

  root.dispatchEvent(new Event('pointerdown'));
  (root as unknown as FakeElement).click();
  resolveLoad();
  await pending;
  await flushPromises();

  assert.equal(loads, 1);
});

test('keyboard input preloads without cancelling or synthesizing a click', async () => {
  const root = new FakeElement() as unknown as Element;
  let loads = 0;
  installInteractionHydration({
    load: async () => {
      loads++;
    },
    root,
  });
  root.addEventListener('click', () => {
    (root as unknown as FakeElement).appClicks++;
  });

  const keydown = new Event('keydown', { bubbles: true, cancelable: true });
  root.dispatchEvent(keydown);
  await flushPromises();

  assert.equal(keydown.defaultPrevented, false);
  assert.equal(loads, 1);
  assert.equal((root as unknown as FakeElement).appClicks, 0);
});

test('modified clicks pass through unchanged while preloading', async () => {
  const root = new FakeElement() as unknown as Element;
  let loads = 0;
  installInteractionHydration({
    load: async () => {
      loads++;
    },
    root,
  });
  root.addEventListener('click', () => {
    (root as unknown as FakeElement).appClicks++;
  });

  const click = new FakeMouseEvent('click', {
    bubbles: true,
    cancelable: true,
    ctrlKey: true,
  });
  root.dispatchEvent(click);
  await flushPromises();

  assert.equal(click.defaultPrevented, false);
  assert.equal(loads, 1);
  assert.equal((root as unknown as FakeElement).appClicks, 1);
});

test('previously cancelled clicks are not replayed', async () => {
  const root = new FakeElement() as unknown as Element;
  let loads = 0;
  installInteractionHydration({
    load: async () => {
      loads++;
    },
    root,
  });
  root.addEventListener('click', () => {
    (root as unknown as FakeElement).appClicks++;
  });

  const click = new FakeMouseEvent('click', {
    bubbles: true,
    cancelable: true,
  });
  click.preventDefault();
  root.dispatchEvent(click);
  await flushPromises();

  assert.equal(loads, 1);
  assert.equal((root as unknown as FakeElement).appClicks, 1);
});

test('unclonable clicks pass through instead of losing native behavior', async () => {
  const root = new FakeElement() as unknown as Element;
  let loads = 0;
  installInteractionHydration({
    load: async () => {
      loads++;
    },
    root,
  });
  root.addEventListener('click', () => {
    (root as unknown as FakeElement).appClicks++;
  });

  const click = new Event('click', { bubbles: true, cancelable: true });
  root.dispatchEvent(click);
  await flushPromises();

  assert.equal(click.defaultPrevented, false);
  assert.equal(loads, 1);
  assert.equal((root as unknown as FakeElement).appClicks, 1);
});

test('fallback replay bypasses a replacement boundary', async () => {
  const root = new FakeElement() as unknown as Element;
  let replacementLoads = 0;
  installInteractionHydration({
    load: async () => {
      throw new Error('chunk unavailable');
    },
    onError: () => {
      installInteractionHydration({
        load: async () => {
          replacementLoads++;
        },
        root,
      });
    },
    root,
  });
  root.addEventListener('click', () => {
    (root as unknown as FakeElement).appClicks++;
  });

  (root as unknown as FakeElement).click();
  await flushPromises();

  assert.equal(replacementLoads, 0);
  assert.equal((root as unknown as FakeElement).appClicks, 1);
});

test('replay still waits for an unvisited nested boundary', async () => {
  const outer = new FakeElement() as unknown as Element;
  const inner = new FakeElement() as unknown as Element;
  let innerLoads = 0;
  installInteractionHydration({
    load: async () => undefined,
    root: outer,
  });
  installInteractionHydration({
    load: async () => {
      innerLoads++;
    },
    root: inner,
  });
  inner.addEventListener('click', () => {
    (inner as unknown as FakeElement).appClicks++;
  });

  const click = new FakeMouseEvent('click', {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(click, 'composedPath', {
    value: () => [inner, outer],
  });
  outer.dispatchEvent(click);
  await flushPromises();

  assert.equal(innerLoads, 1);
  assert.equal((inner as unknown as FakeElement).appClicks, 1);
});

test('replays on the first target in the composed path', async () => {
  const root = new FakeElement() as unknown as Element;
  const innerTarget = new FakeElement();
  let innerClicks = 0;
  innerTarget.addEventListener('click', () => {
    innerClicks++;
  });
  installInteractionHydration({
    load: async () => undefined,
    root,
  });

  const click = new FakeMouseEvent('click', {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(click, 'composedPath', {
    value: () => [innerTarget, root],
  });
  root.dispatchEvent(click);
  await flushPromises();

  assert.equal(innerClicks, 1);
});

test('load failure is reported and replays native behavior once', async () => {
  const root = new FakeElement() as unknown as Element;
  const failure = new Error('chunk unavailable');
  let reported: unknown;
  let loads = 0;
  installInteractionHydration({
    load: async () => {
      loads++;
      throw failure;
    },
    onError: (error) => {
      reported = error;
    },
    root,
  });
  root.addEventListener('click', () => {
    (root as unknown as FakeElement).appClicks++;
  });

  (root as unknown as FakeElement).click();
  await flushPromises();
  (root as unknown as FakeElement).click();

  assert.equal(loads, 1);
  assert.equal(reported, failure);
  assert.equal((root as unknown as FakeElement).appClicks, 2);
});

test('disposer removes the boundary before it loads', async () => {
  const root = new FakeElement() as unknown as Element;
  let loads = 0;
  const dispose = installInteractionHydration({
    load: async () => {
      loads++;
    },
    root,
  });
  dispose();

  (root as unknown as FakeElement).click();
  await flushPromises();

  assert.equal(loads, 0);
});

test('successful hydration permits a later boundary on the same root', async () => {
  const root = new FakeElement() as unknown as Element;
  let firstLoads = 0;
  let secondLoads = 0;
  installInteractionHydration({
    load: async () => {
      firstLoads++;
    },
    root,
  });
  (root as unknown as FakeElement).click();
  await flushPromises();

  installInteractionHydration({
    load: async () => {
      secondLoads++;
    },
    root,
  });
  (root as unknown as FakeElement).click();
  await flushPromises();

  assert.equal(firstLoads, 1);
  assert.equal(secondLoads, 1);
});

test('throwing error callback cannot prevent native replay fallback', async () => {
  const root = new FakeElement() as unknown as Element;
  const previousConsoleError = console.error;
  let loggedErrors = 0;
  console.error = () => {
    loggedErrors++;
  };
  try {
    installInteractionHydration({
      load: async () => {
        throw new Error('chunk unavailable');
      },
      onError: () => {
        throw new Error('reporter unavailable');
      },
      root,
    });
    root.addEventListener('click', () => {
      (root as unknown as FakeElement).appClicks++;
    });

    (root as unknown as FakeElement).click();
    await flushPromises();

    assert.equal((root as unknown as FakeElement).appClicks, 1);
    assert.equal(loggedErrors, 1);
  } finally {
    console.error = previousConsoleError;
  }
});
