// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import '../../../src/template-cache-entry.js';
import '../../../src/lazy-entry.js';
import { attr, getTemplate, observable, WebUIElement } from '../../../src/index.js';
import { collectTemplates } from '../../../src/template-cache.js';

startRuntime();

export class LazyOwner extends WebUIElement {
  @attr label = '';
  @attr({ mode: 'boolean' }) opened = false;
  @observable message = '';
  @observable count = 0;
  @observable items: { id: string; label: string }[] = [];
  @observable expanded = false;
  @observable advanced = false;
  @observable single = false;
  @observable loading = false;
  @observable error = '';
  @observable chosen = '';
  inputEl!: HTMLInputElement;
  singleEl?: HTMLInputElement;
  private generation = 0;
  private request: AbortController | undefined;

  /** Fetch application state and activate the body; the framework loads its pack. */
  async open(): Promise<void> {
    const generation = ++this.generation;
    this.request?.abort();
    const controller = new AbortController();
    this.request = controller;
    this.loading = true;
    this.error = '';
    const query = new URLSearchParams(window.location.search);
    query.set('owner', this.localName);
    try {
      const response = await fetch(`/lazy-ui/state?${query}`, { signal: controller.signal });
      if (!response.ok) throw new Error(`could not load lazy state (${response.status})`);
      const state = await response.json() as Record<string, unknown>;
      if (generation !== this.generation || controller.signal.aborted) return;
      this.setState(state);
      this.opened = true;
      await this.$rendered();
    } catch (error) {
      if (generation === this.generation && !controller.signal.aborted) this.error = String(error);
    } finally {
      if (generation === this.generation) this.loading = false;
    }
  }

  close(): void {
    this.generation++;
    this.request?.abort();
    this.request = undefined;
    this.opened = false;
    this.loading = false;
    this.message = '';
    this.items = [];
    this.chosen = '';
  }

  increment(): void { this.count++; }
  pick(id: string): void { this.chosen = id; }
}

export class LazyShadow extends LazyOwner {}
export class LazyLight extends LazyOwner {}
export class LazyDialog extends LazyOwner {
  dialogEl!: HTMLDialogElement;

  override hydratedCallback(): void {
    if (this.opened && !this.dialogEl.open) this.dialogEl.showModal();
  }

  override async open(): Promise<void> {
    if (!this.dialogEl.open) this.dialogEl.showModal();
    await super.open();
  }

  override close(): void {
    super.close();
    this.dialogEl.close();
  }

  onNativeClosed(): void {
    if (this.opened || this.loading) super.close();
  }
}
LazyShadow.define('test-lazy-shadow');
LazyLight.define('test-lazy-light');
LazyDialog.define('test-lazy-dialog');

declare global {
  interface Window {
    __lazyFixture: {
      collect(): number;
      registered(owner: string, ordinal?: number): boolean;
      packUrl(owner: string, ordinal?: number): string;
      inventory(): string;
      track(owner: string): void;
      retained(): boolean;
    };
  }
}
let observed: WeakRef<object> | undefined;
window.__lazyFixture = {
  collect: collectTemplates,
  registered: (owner, ordinal = 0) => window.__webui?.templates?.[`${owner}::lazy:@${ordinal}`] !== undefined,
  packUrl(owner, ordinal = 0) {
    const region = `${owner}::lazy:@${ordinal}`;
    const templates = window.__webui?.templates ?? {};
    for (const name in templates) {
      const url = templates[name].lp?.[region];
      if (url) return new URL(url, document.baseURI).href;
    }
    return '';
  },
  inventory: () => window.__webui?.inventory ?? '',
  track(owner) {
    const metadata = getTemplate(`${owner}::lazy:@0`);
    if (!metadata) throw new Error('The observed lazy body must be loaded.');
    observed = new WeakRef(metadata);
  },
  retained: () => observed?.deref() !== undefined,
};
