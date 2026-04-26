// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";

export class DocsThemeToggle extends WebUIElement {
  @observable icon = "🌙";

  connectedCallback(): void {
    super.connectedCallback();
    const stored = localStorage.getItem("theme");
    if (stored) {
      document.documentElement.setAttribute("data-theme", stored);
      this.icon = stored === "dark" ? "☀️" : "🌙";
    } else if (window.matchMedia("(prefers-color-scheme: dark)").matches) {
      this.icon = "☀️";
    }
  }

  toggle(): void {
    const html = document.documentElement;
    const current = html.getAttribute("data-theme");
    const isDark =
      current === "dark" ||
      (!current && window.matchMedia("(prefers-color-scheme: dark)").matches);
    const next = isDark ? "light" : "dark";
    html.setAttribute("data-theme", next);
    localStorage.setItem("theme", next);
    this.icon = next === "dark" ? "☀️" : "🌙";
  }
}

DocsThemeToggle.define("docs-theme-toggle");
