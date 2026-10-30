// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

(() => {
  const config = JSON.parse(document.getElementById("webui-press-appearance").textContent);
  const restore = () => {
    const savedTheme = localStorage.getItem(config.storage.theme);
    const savedMode = localStorage.getItem(config.storage.mode) ??
      localStorage.getItem("theme");
    return {
      theme: config.themes.some(family => family.id === savedTheme) ? savedTheme : config.defaultTheme,
      mode: ["system", "light", "dark"].includes(savedMode) ? savedMode : config.defaultMode,
    };
  };
  const warn = error => {
    console.warn("[WebUI Press] Appearance storage is unavailable; changes last for this page session.", error);
  };
  let initial = { theme: config.defaultTheme, mode: config.defaultMode };
  try {
    initial = restore();
  } catch (error) {
    warn(error);
  }
  const root = document.documentElement;
  const media = matchMedia("(prefers-color-scheme: dark)");
  const apply = ({ theme, mode }) => {
    root.setAttribute("data-press-theme", theme);
    root.setAttribute("data-theme-mode", mode);
    root.setAttribute("data-theme", mode === "system"
      ? (media.matches ? "dark" : "light")
      : mode);
  };
  apply(initial);
  const followSystem = () => {
    if (root.getAttribute("data-theme-mode") === "system") {
      root.setAttribute("data-theme", media.matches ? "dark" : "light");
    }
  };
  const followStorage = event => {
    if (event.key !== null && event.key !== config.storage.theme &&
      event.key !== config.storage.mode && event.key !== "theme") return;
    try {
      if (event.storageArea !== null && event.storageArea !== localStorage) return;
      apply(restore());
    } catch (error) {
      warn(error);
    }
  };
  media.addEventListener("change", followSystem);
  window.addEventListener("storage", followStorage);
  // Custom headers may omit all controls. Keep preferences live until the API adopts them.
  document[Symbol.for("@microsoft/webui-press/appearance-bootstrap")] = {
    media,
    stop: () => {
      media.removeEventListener("change", followSystem);
      window.removeEventListener("storage", followStorage);
    },
  };
})();
