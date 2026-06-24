// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";

interface SearchEntry {
  title: string;
  path: string;
  content: string;
  headings?: SearchHeading[];
}

interface SearchHeading {
  text: string;
  anchor: string;
  level: number;
}

interface RankedSearchEntry {
  entry: SearchEntry;
  heading?: SearchHeading;
  path: string;
  score: number;
  snippet: string;
  sortTitle: string;
}

interface HighlightSegment {
  text: string;
  code: boolean;
  mark: boolean;
  className: string;
}

interface SearchResultView {
  path: string;
  className: string;
  titleSegments: HighlightSegment[];
  snippetSegments: HighlightSegment[];
}

export class DocsSearch extends WebUIElement {
  searchInput!: HTMLInputElement;
  dialog!: HTMLDialogElement;

  @observable resultItems: SearchResultView[] = [];
  @observable emptyMessage = "";
  @observable hasResults = false;

  private index: SearchEntry[] | null = null;
  private activeIdx = -1;
  // Each input change clears nested <for> result rows before re-inserting the
  // next set. The version guard drops stale microtasks from rapid typing.
  private renderVersion = 0;

  // Keep title matches decisively above heading matches, and heading matches
  // above body/path matches. Sorting remains deterministic via title/path ties.
  private static readonly TITLE_WEIGHT = 10000;
  private static readonly HEADING_WEIGHT = 1000;
  private static readonly CONTENT_WEIGHT = 100;
  private static readonly PATH_WEIGHT = 50;

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
      if (e.key === "ArrowDown") {
        e.preventDefault();
        this.setActiveIdx(
          Math.min(this.activeIdx + 1, this.resultItems.length - 1),
        );
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        this.setActiveIdx(Math.max(this.activeIdx - 1, 0));
      }
      if (e.key === "Enter" && this.resultItems[this.activeIdx]) {
        window.location.href = this.resultItems[this.activeIdx].path;
      }
    });
  }

  openSearch(): void {
    this.dialog.showModal();
    this.setEmpty("Type to search...");
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
    const query = this.searchInput.value.trim();
    if (!this.index || !query) {
      this.setEmpty("Type to search...");
      return;
    }
    const q = query.toLowerCase();
    const matches = this.rankMatches(q);

    if (matches.length === 0) {
      this.setEmpty("No results found");
      return;
    }

    this.scheduleResults(matches, q);
  }

  private setEmpty(message: string): void {
    this.renderVersion += 1;
    this.resultItems = [];
    this.hasResults = false;
    this.emptyMessage = message;
  }

  private scheduleResults(matches: RankedSearchEntry[], q: string): void {
    const version = this.prepareResultReset();
    queueMicrotask(() => {
      if (version !== this.renderVersion) return;
      this.activeIdx = 0;
      this.setResultsNow(matches, q);
    });
  }

  private prepareResultReset(): number {
    this.renderVersion += 1;
    this.resultItems = [];
    this.hasResults = false;
    this.emptyMessage = "";
    return this.renderVersion;
  }

  private setResultsNow(matches: RankedSearchEntry[], q: string): void {
    const resultItems: SearchResultView[] = [];
    for (let i = 0; i < matches.length; i += 1) {
      const match = matches[i];
      resultItems.push({
        path: match.path,
        className: this.resultClassName(i),
        titleSegments: this.buildTitleSegments(match, q),
        snippetSegments: this.highlightSegments(match.snippet, q, false),
      });
    }
    this.emptyMessage = "";
    this.hasResults = true;
    this.resultItems = resultItems;
  }

  private rankMatches(q: string): RankedSearchEntry[] {
    const ranked: RankedSearchEntry[] = [];
    const index = this.index;
    if (!index) return ranked;

    for (let i = 0; i < index.length; i += 1) {
      const entry = index[i];
      const pageScore = this.scorePage(entry, q);
      if (pageScore > 0) {
        ranked.push({
          entry,
          path: entry.path,
          score: pageScore,
          snippet: this.buildSnippet(entry.content, q),
          sortTitle: entry.title,
        });
      }
      this.pushHeadingMatches(ranked, entry, q);
    }

    ranked.sort((a, b) => {
      if (b.score !== a.score) return b.score - a.score;
      const titleOrder = a.sortTitle.localeCompare(b.sortTitle);
      if (titleOrder !== 0) return titleOrder;
      return a.path.localeCompare(b.path);
    });
    if (ranked.length > 10) {
      ranked.length = 10;
    }
    return ranked;
  }

  private pushHeadingMatches(
    ranked: RankedSearchEntry[],
    entry: SearchEntry,
    q: string,
  ): void {
    const headings = entry.headings;
    if (!headings) return;

    for (let i = 0; i < headings.length; i += 1) {
      const heading = headings[i];
      if (heading.level <= 1) continue;
      const score = this.scoreText(
        heading.text,
        q,
        this.headingWeight(heading.level),
      );
      if (score <= 0) continue;
      ranked.push({
        entry,
        heading,
        path: this.headingPath(entry, heading),
        score,
        snippet: this.buildSnippet(entry.content, q),
        sortTitle: entry.title + " > " + heading.text,
      });
    }
  }

  private scorePage(entry: SearchEntry, q: string): number {
    return (
      this.scoreText(entry.title, q, DocsSearch.TITLE_WEIGHT) +
      this.scoreText(entry.content, q, DocsSearch.CONTENT_WEIGHT) +
      this.scoreText(entry.path, q, DocsSearch.PATH_WEIGHT)
    );
  }

  private headingWeight(level: number): number {
    return Math.max(
      DocsSearch.HEADING_WEIGHT - Math.max(0, level - 2) * 100,
      DocsSearch.CONTENT_WEIGHT + 1,
    );
  }

  private headingPath(entry: SearchEntry, heading: SearchHeading): string {
    if (!heading.anchor) {
      return entry.path;
    }
    return entry.path + "#" + heading.anchor;
  }

  private scoreText(text: string, q: string, weight: number): number {
    const lower = text.toLowerCase();
    const idx = lower.indexOf(q);
    if (idx < 0) return 0;
    if (lower === q) return weight * 4;
    if (idx === 0) return weight * 3;
    return weight * 2 - Math.min(idx, weight);
  }

  private buildSnippet(content: string, q: string): string {
    const lower = content.toLowerCase();
    const idx = lower.indexOf(q);
    if (idx < 0) {
      return this.truncateSnippet(content, 100);
    }
    const start = Math.max(0, idx - 40);
    const end = Math.min(content.length, idx + q.length + 60);
    return (
      (start > 0 ? "..." : "") +
      content.slice(start, end) +
      (end < content.length ? "..." : "")
    );
  }

  private truncateSnippet(content: string, maxLength: number): string {
    if (content.length <= maxLength) {
      return content;
    }
    return content.slice(0, maxLength) + "...";
  }

  private setActiveIdx(idx: number): void {
    if (this.resultItems.length === 0) {
      this.activeIdx = -1;
      return;
    }
    this.activeIdx = idx;
    const updated: SearchResultView[] = [];
    for (let i = 0; i < this.resultItems.length; i += 1) {
      updated.push({
        ...this.resultItems[i],
        className: this.resultClassName(i),
      });
    }
    this.resultItems = updated;
  }

  private resultClassName(index: number): string {
    return index === this.activeIdx ? "result active" : "result";
  }

  private buildTitleSegments(
    match: RankedSearchEntry,
    q: string,
  ): HighlightSegment[] {
    const segments = this.inlineCodeSegments(match.entry.title, q);
    if (match.heading) {
      // Use a dedicated separator segment with CSS margins instead of " > ";
      // browser whitespace collapsing makes leading/trailing inline spaces
      // unreliable once template whitespace is zeroed out.
      this.pushSeparatorSegment(segments);
      this.pushHighlightedSegments(segments, match.heading.text, q, false);
    }
    return segments;
  }

  private inlineCodeSegments(text: string, q: string): HighlightSegment[] {
    const segments: HighlightSegment[] = [];
    let cursor = 0;
    while (cursor < text.length) {
      const start = text.indexOf("`", cursor);
      if (start < 0) {
        this.pushHighlightedSegments(segments, text.slice(cursor), q, false);
        return segments;
      }

      this.pushHighlightedSegments(segments, text.slice(cursor, start), q, false);
      const markerLength = this.countBackticks(text, start);
      const marker = "`".repeat(markerLength);
      const codeStart = start + markerLength;
      const end = text.indexOf(marker, codeStart);
      if (end < 0) {
        this.pushHighlightedSegments(segments, text.slice(start), q, false);
        return segments;
      }

      this.pushHighlightedSegments(
        segments,
        text.slice(codeStart, end),
        q,
        true,
      );
      cursor = end + markerLength;
    }
    return segments;
  }

  private highlightSegments(
    text: string,
    q: string,
    code: boolean,
  ): HighlightSegment[] {
    const segments: HighlightSegment[] = [];
    this.pushHighlightedSegments(segments, text, q, code);
    return segments;
  }

  private pushHighlightedSegments(
    segments: HighlightSegment[],
    text: string,
    q: string,
    code: boolean,
  ): void {
    if (!text) return;
    const lower = text.toLowerCase();
    let cursor = 0;
    while (cursor < text.length) {
      const idx = lower.indexOf(q, cursor);
      if (idx < 0) {
        this.pushSegment(segments, text.slice(cursor), code, false);
        return;
      }
      this.pushSegment(segments, text.slice(cursor, idx), code, false);
      this.pushSegment(segments, text.slice(idx, idx + q.length), code, true);
      cursor = idx + q.length;
    }
  }

  private pushSegment(
    segments: HighlightSegment[],
    text: string,
    code: boolean,
    mark: boolean,
  ): void {
    if (text.length > 0) {
      segments.push({
        text,
        code,
        mark,
        className: mark ? "result-segment result-highlight" : "result-segment",
      });
    }
  }

  private pushSeparatorSegment(segments: HighlightSegment[]): void {
    segments.push({
      text: ">",
      code: false,
      mark: false,
      className: "result-segment result-separator",
    });
  }

  private countBackticks(text: string, start: number): number {
    let cursor = start;
    while (cursor < text.length && text[cursor] === "`") {
      cursor += 1;
    }
    return cursor - start;
  }
}

DocsSearch.define("docs-search");
