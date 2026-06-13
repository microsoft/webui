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
  editing: boolean;
  locked: boolean;
  pending: boolean;
  editValue: string;
  placeholder: string;
}

interface SerializedFileEntry {
  name: string;
  content: string;
}

interface LoadedPlayground {
  files: FileEntry[];
  entry: string;
  active: string;
  stateFile: string;
}

const STATE_FILE = "state.json";
const ENTRY_FILE = "index.html";
const SHARE_PARAM = "playground";
const SHARE_VERSION = 1;
const DEFAULT_NEW_FILE_STEM = "new-component";
const DEFAULT_NEW_FILE = `${DEFAULT_NEW_FILE_STEM}-1.html`;
const COMPANION_EXTENSIONS = ["html", "css"];
const EDITABLE_EXTENSIONS = new Set(COMPANION_EXTENSIONS);
const BASE64_CHUNK_SIZE = 0x8000;

interface PlaygroundData {
  entry?: string;
  files: SerializedFileEntry[];
}

interface SharedPlaygroundPayload {
  v: number;
  entry: string;
  active: string;
  stateFile: string;
  files: SerializedFileEntry[];
}

function createFileEntry(
  name: string,
  content: string,
  active: boolean,
  locked = false,
): FileEntry {
  return {
    name,
    active,
    content,
    editing: false,
    locked,
    pending: false,
    editValue: "",
    placeholder: "",
  };
}

function orderFiles(
  files: FileEntry[],
  entry: string,
  stateFile: string,
): FileEntry[] {
  const ordered: FileEntry[] = [];
  const pinned = new Set<string>();
  for (const name of [entry, stateFile]) {
    if (!name || pinned.has(name)) continue;
    const file = files.find((f) => f.name === name);
    if (file) {
      ordered.push(file);
      pinned.add(name);
    }
  }
  for (const file of files) {
    if (!pinned.has(file.name)) ordered.push(file);
  }
  return ordered;
}

function fileNameParts(name: string): { stem: string; extension: string } {
  const slash = Math.max(name.lastIndexOf("/"), name.lastIndexOf("\\"));
  const dot = name.lastIndexOf(".");
  if (dot > slash) {
    return { stem: name.slice(0, dot), extension: name.slice(dot) };
  }
  return { stem: name, extension: "" };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function encodeBase64Url(value: unknown): string {
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  const chunks: string[] = [];
  for (let i = 0; i < bytes.length; i += BASE64_CHUNK_SIZE) {
    chunks.push(
      String.fromCharCode(...bytes.subarray(i, i + BASE64_CHUNK_SIZE)),
    );
  }
  return btoa(chunks.join(""))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "");
}

function decodeBase64Url(value: string): unknown {
  const base64 = value.replace(/-/g, "+").replace(/_/g, "/");
  const padding = "=".repeat((4 - (base64.length % 4)) % 4);
  const binary = atob(base64 + padding);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return JSON.parse(new TextDecoder().decode(bytes)) as unknown;
}

function normalizeSharedFiles(value: unknown): LoadedPlayground | null {
  if (!isRecord(value)) {
    throw new Error("Shared playground payload must be an object.");
  }
  if (value.v !== SHARE_VERSION) {
    throw new Error("Shared playground payload version is not supported.");
  }
  if (!Array.isArray(value.files) || value.files.length === 0) {
    throw new Error("Shared playground payload does not contain files.");
  }

  const seen = new Set<string>();
  const files: FileEntry[] = [];
  for (const item of value.files) {
    if (!isRecord(item)) {
      throw new Error("Shared playground file entry must be an object.");
    }
    if (typeof item.name !== "string" || typeof item.content !== "string") {
      throw new Error("Shared playground file entries need name and content.");
    }
    const name = item.name.trim();
    if (!name) {
      throw new Error("Shared playground file names cannot be empty.");
    }
    if (seen.has(name)) {
      throw new Error(`Shared playground contains duplicate file "${name}".`);
    }
    seen.add(name);
    files.push(createFileEntry(name, item.content, false));
  }

  const hasEntryFile = seen.has(ENTRY_FILE);
  const hasStateFile = seen.has(STATE_FILE);
  const entry = hasEntryFile
    ? ENTRY_FILE
    : typeof value.entry === "string" && seen.has(value.entry)
      ? value.entry
      : files[0].name;
  let stateFile = hasStateFile ? STATE_FILE : "";
  if (!hasStateFile && typeof value.stateFile === "string") {
    stateFile = seen.has(value.stateFile) ? value.stateFile : "";
  }
  const active =
    typeof value.active === "string" && seen.has(value.active)
      ? value.active
      : entry;
  for (const file of files) {
    file.active = file.name === active;
    file.locked = file.name === entry || file.name === stateFile;
  }
  return {
    files: orderFiles(files, entry, stateFile),
    entry,
    active,
    stateFile,
  };
}

function loadSharedPlayground(): LoadedPlayground | null {
  const encoded = new URLSearchParams(window.location.search).get(SHARE_PARAM);
  if (!encoded) return null;
  return normalizeSharedFiles(decodeBase64Url(encoded));
}

/**
 * Read the playground's initial files from the page render state, which the
 * docs build pipeline populates from the custom page's `stateFile` (see
 * `crates/webui-press/src/types.rs` `CustomPage`). The same object also drives
 * server-side rendering of the tab strip — see `state` flattening in
 * `crates/webui-press/src/content.rs`. Falls back to a single empty entry file
 * if the page was published without a state file.
 */
function loadInitialFiles(): LoadedPlayground {
  const shared = loadSharedPlayground();
  if (shared) return shared;

  const w = window as unknown as {
    __webui?: { state?: PlaygroundData };
  };
  const top = w.__webui?.state;
  if (top && Array.isArray(top.files) && top.files.length > 0) {
    const entry =
      top.entry && top.files.some((f) => f.name === top.entry)
        ? top.entry
        : top.files[0].name;
    const stateFile = top.files.some((f) => f.name === STATE_FILE)
      ? STATE_FILE
      : "";
    const files = top.files.map((f) =>
      createFileEntry(
        f.name,
        typeof f.content === "string" ? f.content : "",
        f.name === entry,
        f.name === entry || f.name === stateFile,
      ),
    );
    return {
      files: orderFiles(files, entry, stateFile),
      entry,
      active: entry,
      stateFile,
    };
  }
  return {
    files: [createFileEntry(ENTRY_FILE, "", true, true)],
    entry: ENTRY_FILE,
    active: ENTRY_FILE,
    stateFile: STATE_FILE,
  };
}

function extOf(name: string): string {
  const i = name.lastIndexOf(".");
  return i >= 0 ? name.slice(i + 1).toLowerCase() : "";
}

export class DocsPlayground extends WebUIElement {
  editorWrap!: HTMLDivElement;

  @observable files: FileEntry[] = [];
  @observable hasStats = false;
  @observable buildMs = "";
  @observable renderMs = "";
  @observable previewSrcdoc = "";
  @observable toastMessage = "";
  @observable previewBadge = "Loading WASM";
  @observable previewBadgeState = "loading";

  // Error panel state. `hasError` toggles the collapsible panel; the rest are
  // the parsed pieces of a build/compile diagnostic (see `setError`).
  @observable hasError = false;
  @observable errorExpanded = true;
  @observable errorSeverity = "error";
  @observable errorTitle = "";
  @observable errorCode = "";
  @observable errorLocation = "";
  @observable errorSnippet = "";
  @observable errorHelp = "";
  @observable errorRaw = "";

  private active: string = ENTRY_FILE;
  private entry: string = ENTRY_FILE;
  private stateFile: string = STATE_FILE;
  private wasm: WasmModule | null = null;
  private editorView: EditorView | null = null;
  private renderTimer: ReturnType<typeof setTimeout> | null = null;
  private toastTimer: ReturnType<typeof setTimeout> | null = null;
  private themeObserver: MutationObserver | null = null;
  private suppressNextEditBlur = false;

  connectedCallback(): void {
    super.connectedCallback();
    let initial: LoadedPlayground;
    try {
      initial = loadInitialFiles();
    } catch (e) {
      initial = {
        files: [createFileEntry(ENTRY_FILE, "", true, true)],
        entry: ENTRY_FILE,
        active: ENTRY_FILE,
        stateFile: STATE_FILE,
      };
      this.setPreviewStatus("Failed", "failed");
      this.setError(`Unable to load shared playground: ${String(e)}`);
    }
    this.files = initial.files;
    this.entry = initial.entry;
    this.active = initial.active;
    this.stateFile = initial.stateFile;
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
    if (this.renderTimer) {
      clearTimeout(this.renderTimer);
      this.renderTimer = null;
    }
    if (this.toastTimer) {
      clearTimeout(this.toastTimer);
      this.toastTimer = null;
    }
    this.editorView?.destroy();
    this.editorView = null;
  }

  // ── File model helpers ─────────────────────────────────────────

  private fileByName(name: string): FileEntry | undefined {
    return this.files.find((f) => f.name === name);
  }

  private editingFile(): FileEntry | undefined {
    return this.files.find((f) => f.editing);
  }

  private isLockedFile(name: string): boolean {
    return name === this.entry || (!!this.stateFile && name === this.stateFile);
  }

  private isStateFile(name: string): boolean {
    return !!this.stateFile && name === this.stateFile;
  }

  private clearFileEdit(file: FileEntry, activeName: string): FileEntry {
    return {
      ...file,
      active: file.name === activeName,
      editing: false,
      locked: this.isLockedFile(file.name),
      pending: false,
      editValue: "",
      placeholder: "",
    };
  }

  private setActive(name: string): void {
    this.active = name;
    this.files = this.files.map((f) => ({ ...f, active: f.name === name }));
  }

  private tabByName(name: string): HTMLElement | null {
    const tabs = this.shadowRoot?.querySelectorAll<HTMLElement>(".tab");
    if (!tabs) return null;
    for (const tab of tabs) {
      if (tab.dataset.name === name) return tab;
    }
    return null;
  }

  private scrollTabIntoView(name: string): void {
    setTimeout(() => {
      this.$flushUpdates();
      this.tabByName(name)?.scrollIntoView({
        block: "nearest",
        inline: "nearest",
      });
    }, 0);
  }

  private flushEditorToFile(): void {
    if (!this.editorView) return;
    const content = this.editorView.state.doc.toString();
    const f = this.fileByName(this.active);
    if (f) f.content = content;
  }

  private uniqueFileName(candidate: string, ignoreName?: string): string {
    const requested = candidate.trim() || DEFAULT_NEW_FILE;
    const existing = new Set(
      this.files.filter((f) => f.name !== ignoreName).map((f) => f.name),
    );
    if (!existing.has(requested)) return requested;

    const { stem, extension } = fileNameParts(requested);
    for (let i = 1; ; i += 1) {
      const next = `${stem}-${i}${extension}`;
      if (!existing.has(next)) return next;
    }
  }

  private nextDefaultFileStem(): string {
    for (let index = 1; ; index += 1) {
      const stem = `${DEFAULT_NEW_FILE_STEM}-${index}`;
      const htmlName = `${stem}.html`;
      const cssName = `${stem}.css`;
      if (this.fileByName(htmlName) && !this.fileByName(cssName)) {
        return stem;
      }
      if (!this.fileByName(htmlName) && !this.fileByName(cssName)) {
        return stem;
      }
    }
  }

  private suggestedNewFileName(): string {
    const stem = this.nextDefaultFileStem();
    const htmlName = `${stem}.html`;
    if (this.fileByName(htmlName)) return `${stem}.css`;
    return htmlName;
  }

  private focusInlineFileInput(selectValue: boolean): void {
    setTimeout(() => {
      this.$flushUpdates();
      const input = this.shadowRoot?.querySelector<HTMLInputElement>(
        ".tab-rename-input",
      );
      input?.focus();
      if (selectValue) input?.select();
    }, 0);
  }

  private commitCurrentFileEditFromDom(): boolean {
    const input = this.shadowRoot?.querySelector<HTMLInputElement>(
      ".tab-rename-input",
    );
    return input ? this.finishFileEdit(input.value) : true;
  }

  // ── File operations (called from template event handlers) ─────

  selectTab(e: Event): void {
    const target = e.target;
    if (target instanceof Element && target.closest(".tab-rename-input")) {
      return;
    }
    const name = (e.currentTarget as HTMLElement).dataset.name;
    if (!name || name === this.active) return;
    if (!this.commitCurrentFileEditFromDom()) return;
    this.flushEditorToFile();
    this.setActive(name);
    this.setupEditor();
    this.scrollTabIntoView(name);
  }

  onTabKeydown(ev: KeyboardEvent): void {
    if (ev.key !== "Enter" && ev.key !== " ") return;
    ev.preventDefault();
    this.selectTab(ev);
  }

  closeFile(e: Event): void {
    e.stopPropagation();
    const name = (e.currentTarget as HTMLElement).dataset.name;
    const file = name ? this.fileByName(name) : undefined;
    if (!file || file.locked) return;
    const closedIndex = this.files.findIndex((f) => f.name === name);
    const remaining = this.files.filter((f) => f.name !== name);
    this.files = remaining;
    if (this.active === name) {
      const nextActive =
        remaining[closedIndex - 1]?.name ??
        remaining[closedIndex]?.name ??
        ENTRY_FILE;
      this.setActive(nextActive);
      this.setupEditor();
      this.scrollTabIntoView(nextActive);
    }
    this.scheduleRender();
  }

  startFileRename(e: Event): void {
    const name = (e.currentTarget as HTMLElement).dataset.name;
    const file = name ? this.fileByName(name) : undefined;
    if (!file || name !== this.active || file.locked) return;
    this.flushEditorToFile();
    this.files = this.files.map((f) =>
      f.name === name
        ? {
            ...f,
            editing: true,
            pending: false,
            editValue: f.name,
            placeholder: f.name,
          }
        : this.clearFileEdit(f, name),
    );
    this.focusInlineFileInput(true);
  }

  openNewFileInput(): void {
    if (!this.commitCurrentFileEditFromDom()) return;
    this.flushEditorToFile();
    const name = this.uniqueFileName(this.suggestedNewFileName());
    const file = createFileEntry(name, "", true);
    file.editing = true;
    file.pending = true;
    file.editValue = name;
    file.placeholder = name;
    this.active = name;
    this.files = [
      ...this.files.map((f) => this.clearFileEdit(f, name)),
      file,
    ];
    this.setupEditor();
    this.scheduleRender();
    this.scrollTabIntoView(name);
    this.focusInlineFileInput(true);
  }

  stopTabEvent(e: Event): void {
    e.stopPropagation();
  }

  onFileEditInput(ev: Event): void {
    const file = this.editingFile();
    if (file) file.editValue = (ev.currentTarget as HTMLInputElement).value;
  }

  onFileEditKey(ev: KeyboardEvent): void {
    ev.stopPropagation();
    if (ev.key === "Enter") {
      ev.preventDefault();
      this.finishFileEdit((ev.currentTarget as HTMLInputElement).value);
    } else if (ev.key === "Escape") {
      ev.preventDefault();
      this.suppressNextEditBlur = true;
      this.cancelFileEdit();
      setTimeout(() => {
        this.suppressNextEditBlur = false;
      }, 0);
    }
  }

  saveFileEdit(ev: Event): void {
    if (this.suppressNextEditBlur) {
      this.suppressNextEditBlur = false;
      return;
    }
    this.finishFileEdit((ev.currentTarget as HTMLInputElement).value);
  }

  private validateEditableFileName(
    name: string,
    editing: FileEntry,
  ): string | null {
    const extension = extOf(name);
    if (!EDITABLE_EXTENSIONS.has(extension)) {
      return "Use a .html or .css file name.";
    }
    if (editing.name === this.entry && extension !== "html") {
      return "The entry file must stay an .html file.";
    }
    return null;
  }

  private rejectFileEdit(message: string): boolean {
    this.showToast(message);
    this.focusInlineFileInput(false);
    return false;
  }

  private finishFileEdit(rawName: string): boolean {
    const editing = this.editingFile();
    if (!editing) return true;
    if (!editing.pending && this.isStateFile(editing.name)) {
      this.cancelFileEdit();
      return true;
    }

    const requested = rawName.trim();
    if (!requested && !editing.pending) {
      this.cancelFileEdit();
      return true;
    }

    const candidate = requested || editing.placeholder || DEFAULT_NEW_FILE;
    const validationError = this.validateEditableFileName(candidate, editing);
    if (validationError) return this.rejectFileEdit(validationError);

    const name = this.uniqueFileName(candidate, editing.name);
    const renamedValidationError = this.validateEditableFileName(
      name,
      editing,
    );
    if (renamedValidationError) {
      return this.rejectFileEdit(renamedValidationError);
    }

    this.flushEditorToFile();
    if (editing.name === this.entry) this.entry = name;
    this.active = name;
    this.files = this.files.map((f) =>
      f.name === editing.name
        ? this.clearFileEdit({ ...f, name }, name)
        : this.clearFileEdit(f, name),
    );
    this.setupEditor();
    this.scheduleRender();
    this.scrollTabIntoView(name);
    return true;
  }

  private cancelFileEdit(): void {
    const editing = this.editingFile();
    if (!editing) return;

    if (editing.pending) {
      const remaining = this.files.filter((f) => f.name !== editing.name);
      const nextActive =
        remaining.find((f) => f.name === this.entry)?.name ??
        remaining[0]?.name ??
        ENTRY_FILE;
      this.active = nextActive;
      if (remaining.length > 0) {
        this.files = remaining.map((f) => this.clearFileEdit(f, nextActive));
      } else {
        this.entry = ENTRY_FILE;
        this.stateFile = STATE_FILE;
        this.files = [createFileEntry(ENTRY_FILE, "", true, true)];
      }
      this.setupEditor();
      this.scheduleRender();
      return;
    }

    this.active = editing.name;
    this.files = this.files.map((f) => this.clearFileEdit(f, editing.name));
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

  // ── Sharing ─────────────────────────────────────────────────────

  async sharePlayground(): Promise<void> {
    if (!this.commitCurrentFileEditFromDom()) return;
    this.flushEditorToFile();

    const payload: SharedPlaygroundPayload = {
      v: SHARE_VERSION,
      entry: this.entry,
      active: this.active,
      stateFile: this.stateFile,
      files: this.files.map((f) => ({
        name: f.name,
        content: f.content,
      })),
    };
    const url = new URL(window.location.href);
    url.searchParams.set(SHARE_PARAM, encodeBase64Url(payload));

    try {
      await this.copyToClipboard(url.toString());
      this.showToast("URL copied to clipboard");
    } catch {
      // A clipboard failure is transient UX noise, not a build error - keep it
      // to a toast rather than the error panel.
      this.showToast("Unable to copy URL");
    }
  }

  private async copyToClipboard(text: string): Promise<void> {
    if (navigator.clipboard?.writeText) {
      try {
        await navigator.clipboard.writeText(text);
        return;
      } catch {
        // Fall back to the hidden textarea path for non-secure contexts.
      }
    }

    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.setAttribute("readonly", "true");
    textarea.style.position = "fixed";
    textarea.style.top = "-1000px";
    textarea.style.left = "-1000px";
    document.body.appendChild(textarea);
    textarea.select();
    const copied = document.execCommand("copy");
    textarea.remove();
    if (!copied) {
      throw new Error("Clipboard copy command failed.");
    }
  }

  private showToast(message: string): void {
    if (this.toastTimer) clearTimeout(this.toastTimer);
    this.toastMessage = message;
    this.toastTimer = setTimeout(() => {
      this.toastMessage = "";
      this.toastTimer = null;
    }, 1800);
  }

  // ── WASM render ─────────────────────────────────────────────────

  private setPreviewStatus(label: string, state: string): void {
    this.previewBadge = label;
    this.previewBadgeState = state;
  }

  /** Reset the error panel to hidden/empty. */
  private clearError(): void {
    this.hasError = false;
    this.errorSeverity = "error";
    this.errorTitle = "";
    this.errorCode = "";
    this.errorLocation = "";
    this.errorSnippet = "";
    this.errorHelp = "";
    this.errorRaw = "";
  }

  /**
   * Parse a build/compile error string into the structured fields the error
   * panel renders. WebUI surfaces authoring diagnostics in a stable shape:
   *
   *   error: <title> [<code>]
   *     --> <file>:<line>:<col>
   *       <offending snippet>
   *     help: <fix>
   *
   * Unstructured failures (e.g. WASM unavailable) fall back to a raw message.
   */
  private setError(raw: string): void {
    this.clearError();
    const text = raw.trim();
    const lines = text.split("\n");
    const first = (lines[0] ?? "").trim();
    const head = /^(error|warning)\b:?\s*(.*)$/i.exec(first);

    if (head) {
      this.errorSeverity =
        head[1].toLowerCase() === "warning" ? "warning" : "error";
      let title = head[2].trim();
      const code = /\[([a-z0-9][a-z0-9-]*)\]\s*$/i.exec(title);
      if (code) {
        this.errorCode = code[1];
        title = title.slice(0, code.index).trim();
      }
      this.errorTitle = title || "Build error";
      for (let i = 1; i < lines.length; i++) {
        const t = lines[i].trim();
        if (!t) continue;
        if (t.startsWith("-->")) {
          this.errorLocation = t.replace(/^-->\s*/, "");
        } else if (/^help:/i.test(t)) {
          this.errorHelp = t.replace(/^help:\s*/i, "");
        } else if (!this.errorSnippet) {
          this.errorSnippet = t;
        }
      }
    } else {
      // Unstructured (e.g. infra) error: show the first line as the summary
      // and any remaining detail in the expandable body.
      this.errorTitle = first || "Error";
      const rest = lines.slice(1).join("\n").trim();
      this.errorRaw = rest;
    }

    this.errorExpanded = true;
    this.hasError = true;
  }

  /** Toggle the expand/collapse state of the error panel. */
  toggleError(): void {
    this.errorExpanded = !this.errorExpanded;
  }

  private scheduleRender(): void {
    if (this.renderTimer) clearTimeout(this.renderTimer);
    if (!this.wasm) {
      this.setPreviewStatus("Loading WASM", "loading");
      return;
    }
    this.setPreviewStatus("Compiling", "compiling");
    this.renderTimer = setTimeout(() => this.doRender(), 150);
  }

  private doRender(): void {
    if (!this.wasm) return;
    try {
      this.setPreviewStatus("Compiling", "compiling");
      this.clearError();
      const filesObj: Record<string, string> = {};
      for (const f of this.files) {
        if (f.name !== this.stateFile) filesObj[f.name] = f.content;
      }

      const t0 = performance.now();
      const proto = this.wasm.build_protocol(filesObj, this.entry);
      const t1 = performance.now();
      const stateJson = this.fileByName(this.stateFile)?.content || "{}";
      const html = this.wasm.render(proto, stateJson, this.entry, "/");
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
      this.setPreviewStatus("WASM Live", "live");
    } catch (e) {
      this.setPreviewStatus("Error", "error");
      this.setError(String(e));
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
      this.setPreviewStatus("Loading WASM", "loading");
      const baseMeta = document.querySelector('meta[name="base"]');
      const base = baseMeta?.getAttribute("content") || "/";
      const mod = await import(/* @vite-ignore */ base + "wasm/webui_wasm.js");
      await mod.default();
      this.wasm = mod;
      this.doRender();
    } catch (e) {
      this.setPreviewStatus("Failed", "failed");
      this.setError(
        'WASM not available. Run "cargo xtask build-wasm" to enable the playground.\n\n' +
          String(e),
      );
    }
  }
}

DocsPlayground.define("docs-playground");
