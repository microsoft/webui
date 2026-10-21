// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { attr, observable, WebUIElement } from '@microsoft/webui-framework';
import { fragments, stateUrl } from '../deferred-state.js';

export class DeferredPopover extends WebUIElement {
  @attr({ mode: 'boolean' }) slowData = false;
  @observable activityReady = false;
  @observable activityLoading = false;
  @observable activityError = '';

  triggerElement!: HTMLButtonElement;
  popoverElement!: HTMLElement;
  private request?: AbortController;
  private generation = 0;

  override hydratedCallback(): void {
    if (this.activityReady) this.popoverElement.showPopover();
  }

  override disconnectedCallback(): void {
    this.generation++;
    this.request?.abort();
    super.disconnectedCallback();
  }

  async preload(): Promise<void> {
    try {
      await fragments.ensureLoaded(this.localName, 'activity');
    } catch (error) {
      console.error('Activity preload failed:', error);
      this.$emit('overlay-notice', 'Preload was unavailable. Opening the overlay will try again.');
    }
  }

  onToggle(event: ToggleEvent): void {
    if (event.newState === 'closed') {
      const focused = this.shadowRoot?.activeElement;
      if (focused && this.popoverElement.contains(focused)) this.triggerElement.focus();
      return;
    }
    if (this.activityReady) return;
    event.preventDefault();
    if (this.activityLoading) this.clear();
    else void this.load();
  }

  onToggled(event: ToggleEvent): void {
    if (event.newState === 'closed' && !this.popoverElement.matches(':popover-open') && this.activityReady) {
      this.clear();
    }
  }

  onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape' && this.activityLoading) {
      event.preventDefault();
      this.clear();
    }
  }

  async load(): Promise<void> {
    this.triggerElement.focus();
    const generation = ++this.generation;
    this.request?.abort();
    const request = new AbortController();
    this.request = request;
    this.activityLoading = true;
    this.activityError = '';
    try {
      const state = await fragments.fetchPartial(this.localName, 'activity', {
        url: stateUrl(this.localName, 'activity', this.slowData),
        signal: request.signal,
      });
      if (generation !== this.generation || request.signal.aborted) return;
      this.setState(state);
      this.activityReady = true;
      this.$flushUpdates();
      this.popoverElement.showPopover({ source: this.triggerElement });
    } catch (error) {
      if (generation === this.generation && !request.signal.aborted) {
        console.error('Activity state failed:', error);
        this.activityError = 'Activity could not be loaded. Please try again.';
      }
    } finally {
      if (generation === this.generation) this.activityLoading = false;
    }
  }

  private clear(): void {
    this.generation++;
    this.request?.abort();
    this.request = undefined;
    this.activityReady = false;
    this.activityLoading = false;
    this.activityError = '';
    this.setState({ activityItems: [], activitySummary: '', activityStamp: '' });
  }
}

DeferredPopover.define('deferred-popover');
