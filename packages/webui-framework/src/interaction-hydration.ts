// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Configuration for loading full component hydration on first interaction. */
export interface InteractionHydrationOptions {
  /**
   * Import, register, and hydrate the component graph.
   *
   * The promise must not resolve until component event listeners are ready.
   */
  load: () => Promise<unknown>;
  /** Surface a component-graph load failure to the application. */
  onError?: (error: unknown) => void;
  /** Root whose interactions wake hydration. */
  root: Element;
}

const PRELOAD_EVENTS = [
  'pointerdown',
  'focusin',
  'keydown',
] as const;
const REPLAY_EVENTS = ['click'] as const;
const installedBoundaries = new WeakMap<Element, () => void>();
const interactionReplayBoundaries = new WeakMap<Event, ReadonlySet<Element>>();

/** Return whether an event was replayed by an interaction hydration boundary. */
export function isInteractionReplay(event: Event): boolean {
  return interactionReplayBoundaries.has(event);
}

/**
 * Install a small interaction boundary that loads the full component graph on
 * demand. An unmodified primary click is replayed after `load()` resolves.
 *
 * @returns A function that removes the boundary without loading components.
 */
export function installInteractionHydration(
  options: InteractionHydrationOptions,
): () => void {
  const { root } = options;
  const existing = installedBoundaries.get(root);
  if (existing) return existing;

  let loading: Promise<void> | undefined;
  let listening = true;
  let disposed = false;

  const remove = (): void => {
    if (!listening) return;
    listening = false;
    for (let i = 0; i < PRELOAD_EVENTS.length; i++) {
      root.removeEventListener(PRELOAD_EVENTS[i], preload, true);
    }
    for (let i = 0; i < REPLAY_EVENTS.length; i++) {
      root.removeEventListener(REPLAY_EVENTS[i], replay, true);
    }
    installedBoundaries.delete(root);
  };

  const dispose = (): void => {
    disposed = true;
    remove();
  };

  const load = (): Promise<void> => {
    loading ??= Promise.resolve().then(options.load).then(
      () => {
        remove();
      },
      (error) => {
        remove();
        reportFailure(error, options.onError);
      },
    );
    return loading;
  };

  const preload = (): void => {
    void load();
  };

  const replay = (event: Event): void => {
    if (
      !listening
      || interactionReplayBoundaries.get(event)?.has(root) === true
    ) {
      return;
    }
    const target = replayTarget(event);
    const mouseEvent = asMouseEvent(event);
    if (!target || !mouseEvent || !shouldReplay(mouseEvent)) {
      preload();
      return;
    }
    const replayEvent = cloneClick(mouseEvent, target, root);
    if (!replayEvent) {
      preload();
      return;
    }
    event.preventDefault();
    event.stopImmediatePropagation();
    const dispatchReplay = (): void => {
      if (!disposed) target.dispatchEvent(replayEvent);
    };
    void load().then(dispatchReplay, dispatchReplay);
  };

  installedBoundaries.set(root, dispose);
  for (let i = 0; i < PRELOAD_EVENTS.length; i++) {
    root.addEventListener(PRELOAD_EVENTS[i], preload, true);
  }
  for (let i = 0; i < REPLAY_EVENTS.length; i++) {
    root.addEventListener(REPLAY_EVENTS[i], replay, true);
  }
  return dispose;
}

function replayTarget(event: Event): EventTarget | null {
  const path = event.composedPath();
  return path.length > 0 ? path[0] : event.target;
}

function asMouseEvent(event: Event): MouseEvent | null {
  const candidate = event as Partial<MouseEvent>;
  return typeof candidate.altKey === 'boolean'
    && typeof candidate.button === 'number'
    && typeof candidate.buttons === 'number'
    && typeof candidate.clientX === 'number'
    && typeof candidate.clientY === 'number'
    && typeof candidate.ctrlKey === 'boolean'
    && typeof candidate.detail === 'number'
    && typeof candidate.metaKey === 'boolean'
    && typeof candidate.screenX === 'number'
    && typeof candidate.screenY === 'number'
    && typeof candidate.shiftKey === 'boolean'
    ? candidate as MouseEvent
    : null;
}

function shouldReplay(event: MouseEvent): boolean {
  if (!event.cancelable || event.defaultPrevented) return false;
  return event.button === 0
    && !event.altKey
    && !event.ctrlKey
    && !event.metaKey
    && !event.shiftKey;
}

function cloneClick(
  event: MouseEvent,
  target: EventTarget,
  boundary: Element,
): MouseEvent | null {
  type EventConstructorWindow = Window & {
    readonly MouseEvent: typeof MouseEvent;
    readonly PointerEvent?: typeof PointerEvent;
  };
  const ownerDocument = 'ownerDocument' in target
    ? (target as EventTarget & { readonly ownerDocument?: Document | null })
        .ownerDocument
    : undefined;
  const view = (
    ownerDocument?.defaultView ?? event.view
  ) as EventConstructorWindow | null;
  const MouseEventConstructor = view?.MouseEvent
    ?? (typeof MouseEvent === 'function' ? MouseEvent : undefined);
  if (!MouseEventConstructor) return null;

  const eventInit: EventInit = {
    bubbles: event.bubbles,
    cancelable: event.cancelable,
    composed: event.composed,
  };
  const mouseInit: MouseEventInit = {
    ...eventInit,
    altKey: event.altKey,
    button: event.button,
    buttons: event.buttons,
    clientX: event.clientX,
    clientY: event.clientY,
    ctrlKey: event.ctrlKey,
    detail: event.detail,
    metaKey: event.metaKey,
    movementX: event.movementX,
    movementY: event.movementY,
    relatedTarget: event.relatedTarget,
    screenX: event.screenX,
    screenY: event.screenY,
    shiftKey: event.shiftKey,
    view,
  };
  const pointer = event as Partial<PointerEvent>;
  const PointerEventConstructor = view?.PointerEvent
    ?? (typeof PointerEvent === 'function' ? PointerEvent : undefined);
  if (
    PointerEventConstructor
    && typeof pointer.pointerId === 'number'
    && typeof pointer.pointerType === 'string'
  ) {
    const replayEvent = new PointerEventConstructor('click', {
      ...mouseInit,
      height: pointer.height,
      isPrimary: pointer.isPrimary,
      pointerId: pointer.pointerId,
      pointerType: pointer.pointerType,
      pressure: pointer.pressure,
      tangentialPressure: pointer.tangentialPressure,
      tiltX: pointer.tiltX,
      tiltY: pointer.tiltY,
      twist: pointer.twist,
      width: pointer.width,
    });
    markInteractionReplay(replayEvent, event, boundary);
    return replayEvent;
  }
  const replayEvent = new MouseEventConstructor('click', mouseInit);
  markInteractionReplay(replayEvent, event, boundary);
  return replayEvent;
}

function markInteractionReplay(
  replay: Event,
  source: Event,
  boundary: Element,
): void {
  const traversed = interactionReplayBoundaries.get(source);
  const boundaries = traversed ? new Set(traversed) : new Set<Element>();
  boundaries.add(boundary);
  interactionReplayBoundaries.set(replay, boundaries);
}

function reportFailure(
  error: unknown,
  onError: ((error: unknown) => void) | undefined,
): void {
  if (onError) {
    try {
      onError(error);
    } catch (onErrorFailure) {
      console.error(
        '[WebUI] interaction hydration error handler failed:',
        onErrorFailure,
      );
    }
    return;
  }
  console.error(
    `[WebUI] interaction hydration failed: ${
      error instanceof Error ? error.message : String(error)
    }`,
  );
}
