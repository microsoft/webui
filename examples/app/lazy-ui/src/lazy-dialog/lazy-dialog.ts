// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { attr, observable, WebUIElement } from '@microsoft/webui-framework';
import { fetchState } from '../state-api.js';

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
      const state = await fetchState('settings', this.slowData, request.signal);
      if (generation !== this.generation || request.signal.aborted) return;
      this.setState(state);
      this.densityDraft = this.compactMode ? 'compact' : 'comfortable';
      this.highlightDraft = this.highlightPinned;
      this.settingsReady = true;
      // showModal() moves focus into the dialog, so its body must exist first.
      // If the body cannot load, onLazyError() resets the dialog before this rejects.
      await this.$rendered();
      if (generation !== this.generation) return;
      this.dialog.showModal();
    } catch (error) {
      if (generation === this.generation && !request.signal.aborted) {
        console.error('Settings failed to load:', error);
        this.settingsReady = false;
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

  /**
   * The settings body could not load its pack or the preview's script.
   *
   * This also covers a dialog rendered open by the server, whose preview
   * script loads during hydration rather than from `open()`.
   */
  onLazyError(event: CustomEvent<{ error: unknown }>): void {
    // Handled here, so the runtime does not report it as an uncaught error.
    event.preventDefault();
    console.error('Settings view failed to load:', event.detail.error);
    // The user dismissed the dialog while its body was loading.
    if (!this.settingsReady) return;
    this.dismiss();
    this.settingsError = 'Settings could not be loaded. Please try again.';
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
