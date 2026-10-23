// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { attr, observable, WebUIElement } from '@microsoft/webui-framework';
import { fragments, stateUrl } from '../lazy-state.js';

export interface ViewPreferences {
  compactMode: boolean;
  highlightPinned: boolean;
}

export class LazyDialog extends WebUIElement {
  @attr({ mode: 'boolean' }) slowData = false;
  @attr({ mode: 'boolean' }) compactMode = false;
  @attr({ mode: 'boolean' }) highlightPinned = true;
  @observable densityDraft = 'comfortable';
  @observable highlightDraft = true;
  @observable settingsReady = false;
  @observable settingsLoading = false;
  @observable settingsError = '';

  triggerElement!: HTMLButtonElement;
  dialog!: HTMLDialogElement;
  private request?: AbortController;
  private generation = 0;

  override hydratedCallback(): void {
    if (this.settingsReady) this.dialog.showModal();
  }

  override disconnectedCallback(): void {
    this.generation++;
    this.request?.abort();
    super.disconnectedCallback();
  }

  async preload(): Promise<void> {
    try {
      await fragments.ensureLoaded(this.localName, 'settings');
    } catch (error) {
      console.error('Settings preload failed:', error);
      this.$emit('overlay-notice', 'Preload was unavailable. Opening the overlay will try again.');
    }
  }

  onTrigger(): void {
    if (this.settingsLoading) this.dismiss();
    else void this.open();
  }

  onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape' && this.settingsLoading) {
      event.preventDefault();
      this.dismiss();
    }
  }

  async open(): Promise<void> {
    if (this.settingsReady) {
      if (!this.dialog.open) this.dialog.showModal();
      return;
    }
    this.triggerElement.focus();
    const generation = ++this.generation;
    this.request?.abort();
    const request = new AbortController();
    this.request = request;
    this.settingsLoading = true;
    this.settingsError = '';
    try {
      const state = await fragments.fetchPartial(this.localName, 'settings', {
        url: stateUrl(this.localName, 'settings', this.slowData),
        signal: request.signal,
      });
      if (generation !== this.generation || request.signal.aborted) return;
      this.setState(state);
      this.densityDraft = this.compactMode ? 'compact' : 'comfortable';
      this.highlightDraft = this.highlightPinned;
      this.settingsReady = true;
      this.$flushUpdates();
      this.dialog.showModal();
    } catch (error) {
      if (generation === this.generation && !request.signal.aborted) {
        console.error('Settings state failed:', error);
        this.settingsError = 'Settings could not be loaded. Please try again.';
      }
    } finally {
      if (generation === this.generation) this.settingsLoading = false;
    }
  }

  dismiss(): void {
    this.dialog.close();
    this.clear();
  }

  onClosed(): void {
    if (!this.dialog.open && this.settingsReady) this.clear();
  }

  changeDensity(event: Event): void {
    const select = event.currentTarget;
    if (select instanceof HTMLSelectElement) this.densityDraft = select.value;
  }

  changeHighlight(event: Event): void {
    const input = event.currentTarget;
    if (input instanceof HTMLInputElement) this.highlightDraft = input.checked;
  }

  apply(event: Event): void {
    event.preventDefault();
    const preferences: ViewPreferences = {
      compactMode: this.densityDraft === 'compact',
      highlightPinned: this.highlightDraft,
    };
    this.$emit('preferences-applied', preferences);
    this.dismiss();
  }

  private clear(): void {
    this.generation++;
    this.request?.abort();
    this.request = undefined;
    this.settingsReady = false;
    this.settingsLoading = false;
    this.settingsError = '';
    this.setState({ layoutOptions: [], settingsNote: '', settingsStamp: '' });
  }
}

LazyDialog.define('lazy-dialog');
