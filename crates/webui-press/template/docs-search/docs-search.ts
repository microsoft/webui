// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from "@microsoft/webui-framework";

interface SearchEntry {
  title: string;
  path: string;
  content: string;
}

export class DocsSearch extends WebUIElement {
  searchInput!: HTMLInputElement;
  resultsEl!: HTMLDivElement;
  dialog!: HTMLDialogElement;

  private index: SearchEntry[] | null = null;
  private activeIdx = -1;

  connectedCallback(): void {
    super.connectedCallback();

    // Close on backdrop click (click on <dialog> itself, not its children)
    this.dialog.addEventListener("click", (e: Event) => {
      if (e.target === this.dialog) this.dialog.close();
    });

    document.addEventListener("keydown", (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        this.openSearch();
      }
      if (!this.dialog.open) return;
      const items = this.resultsEl.querySelectorAll(".result");
      if (e.key === "ArrowDown") {
        e.preventDefault();
        this.activeIdx = Math.min(this.activeIdx + 1, items.length - 1);
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        this.activeIdx = Math.max(this.activeIdx - 1, 0);
      }
      items.forEach((el, i) =>
        el.classList.toggle("active", i === this.activeIdx),
      );
      if (e.key === "Enter" && items[this.activeIdx]) {
        window.location.href = (items[this.activeIdx] as HTMLAnchorElement).href;
      }
    });
  }

  openSearch(): void {
    this.dialog.showModal();
    this.resultsEl.innerHTML =
      '<div class="empty">Type to search...</div>';
    this.activeIdx = -1;
    setTimeout(() => this.searchInput.focus(), 50);
    if (!this.index) {
      const base =
        document.querySelector("meta[name='base']")?.getAttribute("content") ||
        "/";
      fetch(base + "search-index.json")
        .then((r) => r.json())
        .then((data: SearchEntry[]) => {
          this.index = data;
        });
    }
  }

  onInput(): void {
    const query = this.searchInput.value;
    if (!this.index || !query) {
      this.resultsEl.innerHTML =
        '<div class="empty">Type to search...</div>';
      return;
    }
    const q = query.toLowerCase();
    const matches = this.index
      .filter(
        (p) =>
          p.title.toLowerCase().includes(q) ||
          p.content.toLowerCase().includes(q),
      )
      .slice(0, 10);

    if (matches.length === 0) {
      this.resultsEl.innerHTML =
        '<div class="empty">No results found</div>';
      return;
    }

    this.activeIdx = 0;
    this.resultsEl.innerHTML = matches
      .map((m, i) => {
        let snippet = "";
        const idx = m.content.toLowerCase().indexOf(q);
        if (idx >= 0) {
          const start = Math.max(0, idx - 40);
          const end = Math.min(m.content.length, idx + q.length + 60);
          snippet =
            (start > 0 ? "..." : "") +
            m.content.slice(start, end) +
            (end < m.content.length ? "..." : "");
        } else {
          snippet = m.content.slice(0, 100) + "...";
        }
        return (
          '<a class="result' +
          (i === 0 ? " active" : "") +
          '" href="' +
          m.path +
          '"><div class="result-title">' +
          m.title +
          '</div><div class="result-snippet">' +
          snippet +
          "</div></a>"
        );
      })
      .join("");
  }
}

DocsSearch.define("docs-search");
