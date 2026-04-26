// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";

import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightSpecialChars,
} from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { oneDark } from "@codemirror/theme-one-dark";
import {
  bracketMatching,
  syntaxHighlighting,
  defaultHighlightStyle,
} from "@codemirror/language";
import { html as htmlLang } from "@codemirror/lang-html";
import { json as jsonLang } from "@codemirror/lang-json";
import { css as cssLang } from "@codemirror/lang-css";

interface WasmModule {
  build_protocol(files: Record<string, string>, entry: string): string;
  render(protocol: string, state: string, entry: string, path: string): string;
}

interface FileEntry {
  name: string;
  active: boolean;
  content: string;
}

const STATE_FILE = "state.json";
const ENTRY_FILE = "index.html";
const PROTECTED_FILES = new Set([ENTRY_FILE, STATE_FILE]);

interface PlaygroundData {
  entry?: string;
  files: FileEntry[];
}

/**
 * Read the playground's initial files from the page render state, which the
 * docs build pipeline populates from the custom page's `stateFile` (see
 * `crates/webui-press/src/types.rs` `CustomPage`). The same object also drives
 * server-side rendering of the tab strip — see `state` flattening in
 * `crates/webui-press/src/content.rs`. Falls back to a single empty entry file
 * if the page was published without a state file.
 */
function loadInitialFiles(): { files: FileEntry[]; entry: string } {
  const w = window as unknown as {
    __webui?: { state?: { files?: FileEntry[]; entry?: string } };
  };
  const top = w.__webui?.state;
  if (top && Array.isArray(top.files) && top.files.length > 0) {
    const entry =
      top.entry && top.files.some((f) => f.name === top.entry)
        ? top.entry
        : top.files[0].name;
    return {
      files: top.files.map((f) => ({
        name: f.name,
        active: f.name === entry,
        content: typeof f.content === "string" ? f.content : "",
      })),
      entry,
    };
  }
  return {
    files: [{ name: ENTRY_FILE, active: true, content: "" }],
    entry: ENTRY_FILE,
  };
}

function extOf(name: string): string {
  const i = name.lastIndexOf(".");
  return i >= 0 ? name.slice(i + 1) : "";
}

export class DocsPlayground extends WebUIElement {
  editorWrap!: HTMLDivElement;

  @observable files: FileEntry[] = [];
  @observable newFileVisible = false;
  @observable hasStats = false;
  @observable buildMs = "";
  @observable renderMs = "";
  @observable errorMessage = "";
  @observable previewSrcdoc = "";

  private active: string = ENTRY_FILE;
  private wasm: WasmModule | null = null;
  private editorView: EditorView | null = null;
  private renderTimer: ReturnType<typeof setTimeout> | null = null;
  private themeObserver: MutationObserver | null = null;

  connectedCallback(): void {
    super.connectedCallback();
    const initial = loadInitialFiles();
    this.files = initial.files;
    this.active = initial.entry;
    this.setupEditor();
    void this.loadWasm();

    this.themeObserver = new MutationObserver(() => {
      this.setupEditor();
      this.scheduleRender();
    });
    this.themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme", "class"],
    });
  }

  disconnectedCallback(): void {
    super.disconnectedCallback?.();
    this.themeObserver?.disconnect();
    this.themeObserver = null;
    this.editorView?.destroy();
    this.editorView = null;
  }

  // ── File model helpers ─────────────────────────────────────────

  private fileByName(name: string): FileEntry | undefined {
    return this.files.find((f) => f.name === name);
  }

  private setActive(name: string): void {
    this.active = name;
    this.files = this.files.map((f) => ({ ...f, active: f.name === name }));
  }

  private flushEditorToFile(): void {
    if (!this.editorView) return;
    const content = this.editorView.state.doc.toString();
    const f = this.fileByName(this.active);
    if (f) f.content = content;
  }

  // ── File operations (called from template event handlers) ─────

  selectTab(e: Event): void {
    const name = (e.currentTarget as HTMLElement).dataset.name;
    if (!name || name === this.active) return;
    this.flushEditorToFile();
    this.setActive(name);
    this.setupEditor();
  }

  closeFile(e: Event): void {
    e.stopPropagation();
    const name = (e.currentTarget as HTMLElement).dataset.name;
    if (!name || PROTECTED_FILES.has(name)) return;
    this.files = this.files.filter((f) => f.name !== name);
    if (this.active === name) {
      this.setActive(ENTRY_FILE);
      this.setupEditor();
    }
    this.scheduleRender();
  }

  openNewFileInput(): void {
    this.newFileVisible = true;
    setTimeout(() => {
      const input = this.shadowRoot?.querySelector<HTMLInputElement>(
        ".new-file-input",
      );
      input?.focus();
    }, 0);
  }

  cancelNewFile(): void {
    this.newFileVisible = false;
  }

  onNewFileKey(ev: KeyboardEvent): void {
    if (ev.key === "Enter") {
      ev.preventDefault();
      const name = (ev.currentTarget as HTMLInputElement).value.trim();
      if (name && !this.fileByName(name)) {
        this.files = [
          ...this.files.map((f) => ({ ...f, active: false })),
          { name, active: true, content: "" },
        ];
        this.active = name;
        this.setupEditor();
        this.scheduleRender();
      }
      this.cancelNewFile();
    } else if (ev.key === "Escape") {
      this.cancelNewFile();
    }
  }

  // ── Editor ──────────────────────────────────────────────────────

  private isDarkTheme(): boolean {
    const attr = document.documentElement.getAttribute("data-theme");
    if (attr === "dark") return true;
    if (attr === "light") return false;
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  }

  private setupEditor(): void {
    if (this.editorView) {
      this.editorView.destroy();
      this.editorView = null;
    }

    const ext = extOf(this.active);
    const langExt =
      ext === "json" ? jsonLang() : ext === "css" ? cssLang() : htmlLang();

    const isDark = this.isDarkTheme();
    const updateListener = EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        const f = this.fileByName(this.active);
        if (f) f.content = update.state.doc.toString();
        this.scheduleRender();
      }
    });

    this.editorView = new EditorView({
      root: this.shadowRoot!,
      state: EditorState.create({
        doc: this.fileByName(this.active)?.content || "",
        extensions: [
          lineNumbers(),
          highlightActiveLine(),
          highlightSpecialChars(),
          history(),
          bracketMatching(),
          keymap.of([...defaultKeymap, ...historyKeymap]),
          langExt,
          ...(isDark
            ? [oneDark]
            : [syntaxHighlighting(defaultHighlightStyle, { fallback: true })]),
          updateListener,
        ],
      }),
      parent: this.editorWrap,
    });
  }

  // ── WASM render ─────────────────────────────────────────────────

  private scheduleRender(): void {
    if (this.renderTimer) clearTimeout(this.renderTimer);
    this.renderTimer = setTimeout(() => this.doRender(), 150);
  }

  private doRender(): void {
    if (!this.wasm) return;
    try {
      this.errorMessage = "";
      const filesObj: Record<string, string> = {};
      for (const f of this.files) {
        if (f.name !== STATE_FILE) filesObj[f.name] = f.content;
      }

      const t0 = performance.now();
      const proto = this.wasm.build_protocol(filesObj, ENTRY_FILE);
      const t1 = performance.now();
      const stateJson = this.fileByName(STATE_FILE)?.content || "{}";
      const html = this.wasm.render(proto, stateJson, ENTRY_FILE, "/");
      const t2 = performance.now();

      this.buildMs = (t1 - t0).toFixed(1);
      this.renderMs = (t2 - t1).toFixed(1);
      this.hasStats = true;

      let css = "";
      for (const f of this.files) {
        if (f.name.endsWith(".css")) css += f.content + "\n";
      }

      const styles = this.previewStyles();
      this.previewSrcdoc =
        "<!DOCTYPE html><html><head><meta charset='utf-8'>" +
        "<meta name='color-scheme' content='light dark'>" +
        "<style>" +
        styles +
        css +
        "</style></head><body>" +
        html +
        "</body></html>";
    } catch (e) {
      this.errorMessage = String(e);
    }
  }

  private previewStyles(): string {
    const cs = getComputedStyle(document.documentElement);
    const v = (name: string, fallback: string) =>
      cs.getPropertyValue(name).trim() || fallback;
    const bg = v("--docs-color-bg", "#ffffff");
    const text = v("--docs-color-text", "#213547");
    const border = v("--docs-color-border", "#e2e2e3");
    const mono = v(
      "--docs-font-mono",
      "ui-monospace, SFMono-Regular, Menlo, monospace",
    );
    const sans = v(
      "--docs-font-sans",
      "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
    );
    return (
      "*,*::before,*::after{box-sizing:border-box}" +
      `body{font-family:${sans};padding:24px;margin:0;color:${text};background:${bg};line-height:1.6;}` +
      `h1,h2,h3,h4,h5,h6{color:${text};margin-top:0;}` +
      `code,pre{font-family:${mono};}` +
      `hr{border:0;border-top:1px solid ${border};}`
    );
  }

  private async loadWasm(): Promise<void> {
    try {
      const baseMeta = document.querySelector('meta[name="base"]');
      const base = baseMeta?.getAttribute("content") || "/";
      const mod = await import(/* @vite-ignore */ base + "wasm/webui_wasm.js");
      await mod.default();
      this.wasm = mod;
      this.doRender();
    } catch (e) {
      this.errorMessage =
        'WASM not available. Run "cargo xtask build-wasm" to enable the playground.\n\n' +
        String(e);
    }
  }
}

DocsPlayground.define("docs-playground");
