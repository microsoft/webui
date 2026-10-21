// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import '../../../src/template-cache-entry.js';
import '../../../src/deferred-entry.js';
import { attr, getTemplate, observable, WebUIElement } from '../../../src/index.js';
import { createDeferredLoader, deferredFragment } from '../../../../webui-router/src/deferred-entry.js';
import { deferredNdjson } from '../../../../webui-router/src/deferred-ndjson.js';
import { collectTemplates } from '../../../src/template-cache.js';

let moduleLoads = 0;
const loader = createDeferredLoader({
  endpoint: '/deferred-ui/resources',
  transport: deferredNdjson,
  loaders: {
    [deferredFragment('test-deferred-shadow', 'details')]: async () => { moduleLoads++; },
    [deferredFragment('test-deferred-light', 'details')]: async () => { moduleLoads++; },
  },
});

export class DeferredOwner extends WebUIElement {
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

  preload(): void {
    void loader.ensureLoaded(this.localName, 'details').catch(error => {
      this.error = String(error);
    });
  }

  async open(): Promise<void> {
    const generation = ++this.generation;
    this.request?.abort();
    const controller = new AbortController();
    this.request = controller;
    this.loading = true;
    this.error = '';
    const query = new URLSearchParams(window.location.search);
    query.set('owner', this.localName);
    query.set('id', 'details');
    try {
      const state = await loader.fetchPartial(this.localName, 'details', {
        url: `/deferred-ui/state?${query}`,
        signal: controller.signal,
      });
      if (generation !== this.generation || controller.signal.aborted) return;
      this.setState(state);
      this.opened = true;
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

export class DeferredShadow extends DeferredOwner {}
export class DeferredLight extends DeferredOwner {}
export class DeferredDialog extends DeferredOwner {
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
DeferredShadow.define('test-deferred-shadow');
DeferredLight.define('test-deferred-light');
DeferredDialog.define('test-deferred-dialog');

declare global {
  interface Window {
    __deferredFixture: {
      collect(): number;
      moduleLoads(): number;
      track(owner: string): void;
      retained(): boolean;
    };
  }
}
let observed: WeakRef<object> | undefined;
window.__deferredFixture = {
  collect: collectTemplates,
  moduleLoads: () => moduleLoads,
  track(owner) {
    const metadata = getTemplate(deferredFragment(owner, 'details'));
    if (!metadata) throw new Error('The observed deferred body must be loaded.');
    observed = new WeakRef(metadata);
  },
  retained: () => observed?.deref() !== undefined,
};
