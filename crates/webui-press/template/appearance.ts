// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Requested appearance, independent of the selected token family. */
export type AppearanceMode = "system" | "light" | "dark";
/** A configured theme family; token values remain in build-generated CSS. */
export interface ThemeFamily {
  readonly id: string;
  readonly label: string;
}
/** The last applied document appearance. */
export interface Appearance {
  readonly theme: string;
  readonly mode: AppearanceMode;
  readonly resolvedMode: "light" | "dark";
}
/** A partial user selection. Invalid choices reject without changing state. */
export interface AppearanceUpdate {
  theme?: string;
  mode?: AppearanceMode;
}
export type AppearanceListener = (appearance: Appearance) => void;

interface Configuration {
  themes: ThemeFamily[];
  defaultTheme: string;
  defaultMode: AppearanceMode;
  storage: { theme: string; mode: string };
}
interface Owner {
  get(): Appearance;
  families: readonly ThemeFamily[];
  set(update: AppearanceUpdate): Promise<void>;
  subscribe(listener: AppearanceListener): () => void;
}

// Separate bundles (including a consumer's independently bundled npm import)
// share the document owner, not a second set of preferences or event listeners.
const ownerKey = Symbol.for("@microsoft/webui-press/appearance");
const bootstrapKey = Symbol.for("@microsoft/webui-press/appearance-bootstrap");
function owner(): Owner {
  const host = document as Document & { [ownerKey]?: Owner };
  return host[ownerKey] ??= createOwner();
}

function isMode(value: unknown): value is AppearanceMode {
  return value === "system" || value === "light" || value === "dark";
}

function createOwner(): Owner {
  const node = document.getElementById("webui-press-appearance");
  if (!node) throw new Error("WebUI Press appearance requires a full-layout appearance configuration.");
  const config: Configuration = JSON.parse(node.textContent ?? "");
  const families = Object.freeze(config.themes.map(family => Object.freeze({ ...family })));
  const validTheme = (value: unknown): value is string =>
    families.some(family => family.id === value);
  const root = document.documentElement;
  const theme = root.getAttribute("data-press-theme");
  const mode = root.getAttribute("data-theme-mode");
  const resolvedMode = root.getAttribute("data-theme");
  if (!validTheme(theme) || !isMode(mode) ||
    (resolvedMode !== "light" && resolvedMode !== "dark")) {
    throw new Error("WebUI Press appearance initialization did not run. Deploy the generated appearance-init script, allow it in your CSP, and load it before the browser API.");
  }
  let state: Appearance = Object.freeze({ theme, mode, resolvedMode });
  let desired = { theme, mode };
  const host = document as Document & {
    [bootstrapKey]?: { media: MediaQueryList; stop(): void };
  };
  const bootstrap = host[bootstrapKey];
  const media = bootstrap?.media ?? matchMedia("(prefers-color-scheme: dark)");
  bootstrap?.stop();
  delete host[bootstrapKey];
  const listeners = new Set<AppearanceListener>();
  let active: ViewTransition | undefined;
  let pending: Promise<void> | undefined;
  let storageWarning = false;

  function warnStorage(error: unknown): void {
    if (storageWarning) return;
    storageWarning = true;
    console.warn("[WebUI Press] Appearance storage is unavailable; changes last for this page session.", error);
  }

  function apply(persist: boolean): void {
    const next: Appearance = Object.freeze({
      ...desired,
      resolvedMode: desired.mode === "system" ? (media.matches ? "dark" : "light") : desired.mode,
    });
    if (persist) {
      try {
        localStorage.setItem(config.storage.theme, next.theme);
        localStorage.setItem(config.storage.mode, next.mode);
      } catch (error) {
        warnStorage(error);
      }
    }
    if (next.theme === state.theme && next.mode === state.mode &&
      next.resolvedMode === state.resolvedMode) return;
    root.setAttribute("data-press-theme", next.theme);
    root.setAttribute("data-theme-mode", next.mode);
    root.setAttribute("data-theme", next.resolvedMode);
    state = next;
    for (const listener of listeners) {
      // One faulty consumer must not prevent other controls from synchronizing.
      try { listener(state); } catch (error) { reportError(error); }
    }
  }

  function changed(): boolean {
    return desired.theme !== state.theme || desired.mode !== state.mode;
  }

  async function animate(): Promise<void> {
    try {
      while (changed()) {
        const previous = root.getAttribute("data-press-transition");
        root.setAttribute("data-press-transition", "appearance");
        try {
          active = document.startViewTransition(() => apply(true));
          // Attach to all promises immediately. Only ready may reject normally
          // when skipped; update/finished failures must reach the caller.
          const outcomes = await Promise.allSettled([
            active.ready, active.updateCallbackDone, active.finished,
          ]);
          for (const [index, outcome] of outcomes.entries()) {
            if (outcome.status !== "rejected") continue;
            const error: unknown = outcome.reason;
            if (index === 0 && error instanceof DOMException &&
              (error.name === "AbortError" || error.name === "InvalidStateError")) continue;
            throw error;
          }
        } finally {
          active = undefined;
          if (root.getAttribute("data-press-transition") === "appearance") {
            if (previous === null) root.removeAttribute("data-press-transition");
            else root.setAttribute("data-press-transition", previous);
          }
        }
      }
    } catch (error) {
      // Rendering/transition failures must not discard the latest selection.
      apply(true);
      throw error;
    } finally {
      pending = undefined;
    }
  }

  media.addEventListener("change", () => {
    if (desired.mode === "system") {
      active?.skipTransition();
      apply(false);
    }
  });
  window.addEventListener("storage", event => {
    if (event.key !== null && event.key !== config.storage.theme &&
      event.key !== config.storage.mode && event.key !== "theme") return;
    try {
      if (event.storageArea !== null && event.storageArea !== localStorage) return;
      const savedTheme = localStorage.getItem(config.storage.theme);
      const savedMode = localStorage.getItem(config.storage.mode) ?? localStorage.getItem("theme");
      desired = {
        theme: validTheme(savedTheme) ? savedTheme : config.defaultTheme,
        mode: isMode(savedMode) ? savedMode : config.defaultMode,
      };
    } catch (error) {
      warnStorage(error);
      return;
    }
    active?.skipTransition();
    apply(false);
  });
  apply(false);

  return {
    get: () => state,
    families,
    set(update) {
      if (!update || typeof update !== "object" ||
        (update.theme !== undefined && !validTheme(update.theme)) ||
        (update.mode !== undefined && !isMode(update.mode))) {
        return Promise.reject(new TypeError("Choose a configured theme and system, light, or dark appearance."));
      }
      desired = { theme: update.theme ?? desired.theme, mode: update.mode ?? desired.mode };
      if (pending) {
        active?.skipTransition();
        return pending;
      }
      if (!changed()) return Promise.resolve();
      if (!document.startViewTransition ||
        matchMedia("(prefers-reduced-motion: reduce)").matches ||
        matchMedia("(forced-colors: active)").matches) {
        apply(true);
        return Promise.resolve();
      }
      // Deferral also coalesces synchronous inputs before allocating a transition.
      pending = Promise.resolve().then(animate);
      return pending;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}

/** Read the synchronous, applied state (not an animation's pending selection). */
export function getAppearance(): Appearance { return owner().get(); }
/** Read ordered, immutable family metadata. */
export function getThemeFamilies(): readonly ThemeFamily[] { return owner().families; }
/** Apply a validated selection, progressively enhanced with a native transition. */
export async function setAppearance(update: AppearanceUpdate): Promise<void> { return owner().set(update); }
/** Observe applied changes. This does not immediately invoke the listener. */
export function subscribeAppearance(listener: AppearanceListener): () => void {
  return owner().subscribe(listener);
}
