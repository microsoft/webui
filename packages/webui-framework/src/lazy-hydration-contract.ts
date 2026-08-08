// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Tiny contract bridging `WebUIElement` to the optional lazy-hydration
 * coordinator.
 *
 * The real viewport/interaction coordinator (a shared `IntersectionObserver`,
 * activation queue, and wake listeners) lives in
 * `lazy-hydration-coordinator.ts` and is reachable only through the
 * optional `@microsoft/webui-framework/lazy-hydration.js` entry (see
 * `lazy-hydration-entry.ts`). `element.ts` imports only this module, so a
 * build-time component policy never pulls the coordinator into the default
 * framework entry — an application that never imports the optional entry
 * never bundles it. Component policy is compiled from the authored
 * root template, so ordinary components never call anything below beyond a
 * single `undefined` check.
 *
 * Kept dependency-free (no imports) so it can sit underneath both
 * `element.ts` and the coordinator without risking a circular-init race:
 * the coordinator module registers itself here as a side effect of
 * `installLazyHydrationCoordinator()`, which always runs before any
 * component `.define()` body in the same module graph (the optional entry
 * is import-ordered ahead of component modules, mirroring `streaming.js`).
 */

/** Shared activation hook a coordinator invokes on a deferred target. */
export const LAZY_HYDRATION_ACTIVATE = Symbol();
export const LAZY_HYDRATION_VIEWPORT = 1;
export const LAZY_HYDRATION_CONTENT_VISIBILITY = 2;
export type LazyHydrationMode =
  | typeof LAZY_HYDRATION_VIEWPORT
  | typeof LAZY_HYDRATION_CONTENT_VISIBILITY;

export type LazyHydrationTarget = HTMLElement & {
  [LAZY_HYDRATION_ACTIVATE](
    state: Record<string, unknown> | undefined,
  ): void;
};

/** Implemented by `lazy-hydration-coordinator.ts` and installed once. */
export interface LazyHydrationCoordinator {
  /** Whether this browser can defer hydration without risking inert UI. */
  supportsLazyHydration(): boolean;
  /** Register an ordinary SSR component for viewport/interaction activation. */
  observe(target: LazyHydrationTarget, mode: LazyHydrationMode): void;
  /** Register a streamed SSR component only after its boundary commits. */
  observeStreamed(
    target: LazyHydrationTarget,
    state: Record<string, unknown> | undefined,
    mode: LazyHydrationMode,
  ): void;
  /** Stop observing a disconnected component without losing reconnect eligibility. */
  disconnect(target: LazyHydrationTarget): void;
  /** True while a retained streaming activation is entering the normal hydrator. */
  isStreamedActivation(target: LazyHydrationTarget): boolean;
}

let coordinator: LazyHydrationCoordinator | undefined;

/**
 * Install the shared lazy-hydration coordinator. Called once, by
 * `installLazyHydrationCoordinator()` in `lazy-hydration-coordinator.ts`
 * (via the optional `lazy-hydration.js` entry). A later registration
 * replaces the implementation, which supports isolated coordinator tests.
 */
export function registerLazyHydrationCoordinator(
  impl: LazyHydrationCoordinator,
): void {
  coordinator = impl;
}

/** Whether the optional `lazy-hydration.js` entry has been imported. */
export function isLazyHydrationCoordinatorInstalled(): boolean {
  return coordinator !== undefined;
}

/** Whether this browser/coordinator combination can defer hydration safely. */
export function supportsLazyHydration(): boolean {
  return coordinator?.supportsLazyHydration() ?? false;
}

/** Register an ordinary SSR component for viewport and interaction activation. */
export function observeLazyHydration(
  target: LazyHydrationTarget,
  mode: LazyHydrationMode,
): void {
  coordinator?.observe(target, mode);
}

/**
 * Retain boundary-local state and register a streamed SSR component only after
 * its boundary commits.
 */
export function observeStreamedLazyHydration(
  target: LazyHydrationTarget,
  state: Record<string, unknown> | undefined,
  mode: LazyHydrationMode,
): void {
  coordinator?.observeStreamed(target, state, mode);
}

/** Stop observing a disconnected component without losing reconnect eligibility. */
export function disconnectLazyHydration(target: LazyHydrationTarget): void {
  coordinator?.disconnect(target);
}

/** True while a retained streaming activation is entering the normal hydrator. */
export function isStreamedLazyActivation(target: LazyHydrationTarget): boolean {
  return coordinator?.isStreamedActivation(target) ?? false;
}

/** Test-only: drop the registered coordinator between unit tests. */
export function __resetLazyHydrationContractForTests(): void {
  coordinator = undefined;
}
