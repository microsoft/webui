// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Tiny contract bridging `WebUIElement` to the optional visible-hydration
 * coordinator.
 *
 * The real viewport/interaction coordinator (a shared `IntersectionObserver`,
 * activation queue, and wake listeners) lives in
 * `visible-hydration-coordinator.ts` and is reachable only through the
 * optional `@microsoft/webui-framework/visible-hydration.js` entry (see
 * `visible-hydration-entry.ts`). `element.ts` imports only this module, so a
 * `static hydration = 'visible'` opt-in never pulls the coordinator into the
 * default framework entry — an application that never imports the optional
 * entry never bundles it, and a component whose `hydration` stays at the
 * eager default never calls anything below beyond a single `undefined`
 * check.
 *
 * Kept dependency-free (no imports) so it can sit underneath both
 * `element.ts` and the coordinator without risking a circular-init race:
 * the coordinator module registers itself here as a side effect of
 * `installVisibleHydrationCoordinator()`, which always runs before any
 * component `.define()` body in the same module graph (the optional entry
 * is import-ordered ahead of component modules, mirroring `streaming.js`).
 */

/** Shared activation hook a coordinator invokes on a deferred target. */
export const LAZY_HYDRATION_ACTIVATE = Symbol();

export type LazyHydrationTarget = HTMLElement & {
  [LAZY_HYDRATION_ACTIVATE](
    state: Record<string, unknown> | undefined,
  ): void;
};

/** Implemented by `visible-hydration-coordinator.ts` and installed once. */
export interface VisibleHydrationCoordinator {
  /** Whether this browser can defer hydration without risking inert UI. */
  supportsVisibleHydration(): boolean;
  /** Register an ordinary SSR component for viewport/interaction activation. */
  observe(target: LazyHydrationTarget): void;
  /** Register a streamed SSR component only after its boundary commits. */
  observeStreamed(
    target: LazyHydrationTarget,
    state: Record<string, unknown> | undefined,
  ): void;
  /** Stop observing a disconnected component without losing reconnect eligibility. */
  disconnect(target: LazyHydrationTarget): void;
  /** True while a retained streaming activation is entering the normal hydrator. */
  isStreamedActivation(target: LazyHydrationTarget): boolean;
}

let coordinator: VisibleHydrationCoordinator | undefined;

/**
 * Install the shared visible-hydration coordinator. Called once, by
 * `installVisibleHydrationCoordinator()` in `visible-hydration-coordinator.ts`
 * (via the optional `visible-hydration.js` entry). Safe to call again with an
 * equivalent implementation — re-registering is a no-op in practice, since
 * ESM module caching means the installer body itself only runs once per
 * module graph.
 */
export function registerVisibleHydrationCoordinator(
  impl: VisibleHydrationCoordinator,
): void {
  coordinator = impl;
}

/** Whether the optional `visible-hydration.js` entry has been imported. */
export function isVisibleHydrationCoordinatorInstalled(): boolean {
  return coordinator !== undefined;
}

/** Whether this browser/coordinator combination can defer hydration safely. */
export function supportsLazyHydration(): boolean {
  return coordinator?.supportsVisibleHydration() ?? false;
}

/** Register an ordinary SSR component for viewport and interaction activation. */
export function observeLazyHydration(target: LazyHydrationTarget): void {
  coordinator?.observe(target);
}

/**
 * Retain boundary-local state and register a streamed SSR component only after
 * its boundary commits.
 */
export function observeStreamedLazyHydration(
  target: LazyHydrationTarget,
  state: Record<string, unknown> | undefined,
): void {
  coordinator?.observeStreamed(target, state);
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
