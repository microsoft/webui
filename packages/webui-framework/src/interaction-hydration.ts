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
  /** Root whose interactions wake hydration. Omit for one compiler-marked root. */
  root?: Element;
}

const INTERACTION_ROOT_SELECTOR = '[data-webui-interaction]';
const WAKE_EVENTS = [
  'pointerdown',
  'focusin',
  'keydown',
] as const;
interface InstalledBoundary {
  readonly dispose: () => void;
  readonly wake: () => Promise<void>;
}

interface DisposalSlot {
  callback?: () => void;
}

const installedBoundaries = new WeakMap<Element, InstalledBoundary>();
let compilerRoot: WeakRef<Element> | undefined;

interface ReplayTrail {
  readonly boundary: Element;
  readonly previous?: ReplayTrail;
}

const interactionReplays = new WeakMap<Event, ReplayTrail>();

/** Return whether an event was replayed by an interaction hydration boundary. */
export function isInteractionReplay(event: Event): boolean {
  return interactionReplays.has(event);
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
  const root = options.root ?? compilerMarkedRoot();
  const existing = installedBoundaries.get(root);
  if (existing) return existing.dispose;

  let loading: Promise<void> | undefined;
  let listening = true;
  let disposed = false;
  const disposal: DisposalSlot = {};
  const dispose = createDisposer(disposal);

  const remove = (): void => {
    if (!listening) return;
    listening = false;
    for (let i = 0; i < WAKE_EVENTS.length; i++) {
      root.removeEventListener(WAKE_EVENTS[i], wake, true);
    }

    root.removeEventListener('click', replay, true);
    installedBoundaries.delete(root);
    disposal.callback = undefined;
  };

  disposal.callback = () => {
    disposed = true;
    remove();
  };

  const ensureLoaded = (): Promise<void> => {
    loading ??= Promise.resolve().then(options.load).then(
      () => {
        remove();
      },
      (error) => {
        remove();
        reportFailure(error, options.onError);
        throw error;
      },
    );
    return loading;
  };

  const wake = (): void => {
    void ensureLoaded().catch(() => {});
  };

  const replay = (event: Event): void => {
    if (!listening || hasTraversed(event, root)) return;
    const target = event.composedPath()[0] ?? event.target;
    const replayEvent = target ? cloneClick(event, target, root) : null;
    if (!replayEvent) {
      wake();
      return;
    }
    event.preventDefault();
    event.stopImmediatePropagation();
    const dispatchReplay = (): void => {
      if (!disposed) target.dispatchEvent(replayEvent);
    };
    void ensureLoaded().then(dispatchReplay, dispatchReplay);
  };

  installedBoundaries.set(root, { dispose, wake: ensureLoaded });
  for (let i = 0; i < WAKE_EVENTS.length; i++) {
    root.addEventListener(WAKE_EVENTS[i], wake, true);
  }
  root.addEventListener('click', replay, true);
  return dispose;
}

function createDisposer(slot: DisposalSlot): () => void {
  return () => slot.callback?.();
}

/** Begin loading one installed interaction boundary before another wake event. */
export function wakeInteractionHydration(root?: Element): Promise<void> {
  const target = root ?? compilerMarkedRoot();
  const boundary = installedBoundaries.get(target);
  if (!boundary) {
    throw new Error('[WebUI] interaction hydration boundary is not installed.');
  }
  return boundary.wake();
}

function compilerMarkedRoot(): Element {
  const cached = compilerRoot?.deref();
  if (cached && cached.isConnected !== false) return cached;
  const roots = compilerMarkedRoots();
  if (roots.length !== 1) {
    throw new Error(
      `[WebUI] interaction hydration requires exactly one compiler-marked root; found ${roots.length}.`,
    );
  }
  const root = roots[0];
  root.removeAttribute('data-webui-interaction');
  compilerRoot = new WeakRef(root);
  return root;
}

function compilerMarkedRoots(): Element[] {
  const matches: Element[] = [];
  const scopes: ParentNode[] = [document];
  for (let scopeIndex = 0; scopeIndex < scopes.length; scopeIndex++) {
    const scope = scopes[scopeIndex];
    const marked = scope.querySelectorAll(INTERACTION_ROOT_SELECTOR);
    for (let i = 0; i < marked.length; i++) matches.push(marked[i]);
    const elements = scope.querySelectorAll('*');
    for (let i = 0; i < elements.length; i++) {
      const shadowRoot = elements[i].shadowRoot;
      if (shadowRoot) scopes.push(shadowRoot);
    }
  }
  return matches;
}

function cloneClick(
  event: Event,
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
    ownerDocument?.defaultView
  ) as EventConstructorWindow | null;
  const MouseEventConstructor = view?.MouseEvent
    ?? (typeof MouseEvent === 'function' ? MouseEvent : undefined);
  if (
    !MouseEventConstructor
    || !(event instanceof MouseEventConstructor)
    || !event.cancelable
    || event.defaultPrevented
    || event.button !== 0
    || event.altKey
    || event.ctrlKey
    || event.metaKey
    || event.shiftKey
  ) {
    return null;
  }

  const PointerEventConstructor = view?.PointerEvent
    ?? (typeof PointerEvent === 'function' ? PointerEvent : undefined);
  const replayEvent = PointerEventConstructor
      && event instanceof PointerEventConstructor
    ? new PointerEventConstructor('click', event)
    : new MouseEventConstructor('click', event);
  interactionReplays.set(replayEvent, {
    boundary,
    previous: interactionReplays.get(event),
  });
  return replayEvent;
}

function hasTraversed(event: Event, boundary: Element): boolean {
  let trail = interactionReplays.get(event);
  while (trail) {
    if (trail.boundary === boundary) return true;
    trail = trail.previous;
  }
  return false;
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
