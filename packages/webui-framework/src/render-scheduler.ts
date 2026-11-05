// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export const REQUEST_UPDATE = Symbol.for('microsoft.webui.requestUpdate');
export const FLUSH_UPDATES = Symbol.for('microsoft.webui.flushUpdates');
export const RENDER = Symbol('render');
export const RENDER_PENDING = Symbol('render.pending');

export interface RenderTarget extends Element {
  [RENDER_PENDING]: number;
  [RENDER](): void;
}

interface RenderObserver {
  queued(element: Element): void;
  render(element: RenderTarget): void;
  finished(element: Element): void;
}

export const pendingRenders: Array<RenderTarget | undefined> = [];
export let batchingRender = false;
let observer: RenderObserver | undefined;
let scheduled = false;
let cursor = 0;

export function observeRender(observerOrUndefined: RenderObserver | undefined): void {
  observer = observerOrUndefined;
}

export function enqueueRender(element: RenderTarget): void {
  if (!element[RENDER_PENDING]) {
    pendingRenders.push(element);
    element[RENDER_PENDING] = pendingRenders.length;
    if (!scheduled) {
      scheduled = true;
      queueMicrotask(drain);
    }
  }
  observer?.queued(element);
}

export function finishRender(element: Element & { [RENDER_PENDING]?: number }): void {
  const index = element[RENDER_PENDING];
  if (index) {
    pendingRenders[index - 1] = undefined;
    element[RENDER_PENDING] = 0;
  }
  observer?.finished(element);
}

export function flushRender(element: RenderTarget): void {
  const previous = batchingRender;
  batchingRender = false;
  try {
    render(element);
  } finally {
    batchingRender = previous;
  }
}

function render(element: RenderTarget): void {
  if (observer) observer.render(element);
  else element[RENDER]();
}

function drain(): void {
  const previous = batchingRender;
  batchingRender = true;
  try {
    while (cursor < pendingRenders.length) {
      const element = pendingRenders[cursor];
      pendingRenders[cursor++] = undefined;
      if (!element) continue;
      render(element);
    }
  } finally {
    batchingRender = previous;
    scheduled = false;
    if (cursor === pendingRenders.length) {
      pendingRenders.length = 0;
      cursor = 0;
    } else {
      // A failed job is still thrown to the host; other queued work must run.
      scheduled = true;
      queueMicrotask(drain);
    }
  }
}
