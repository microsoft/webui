// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';
import { track } from './ntp-telemetry.js';

/** A selectable color or image swatch. */
export interface Swatch {
  id: string;
  label: string;
  color: string;
}

const TYPEAHEAD_RESET_MS = 700;

/**
 * Keyboard-navigable grid of swatches, shared by the settings flyout and the
 * background gallery. Emits `select` with `{ id }`.
 */
export class NtpSwatchGrid extends WebUIElement {
  @attr label = 'Choices';
  @attr selected = '';
  @observable items: readonly Swatch[] = [];
  @observable hint = '';

  private typed = '';
  private typedAt = 0;

  protected override hydratedCallback(): void {
    this.describe();
  }

  itemsChanged(): void {
    this.describe();
  }

  selectedChanged(): void {
    this.describe();
  }

  choose(event: Event): void {
    const id = (event.currentTarget as HTMLElement | null)?.dataset['id'];
    if (id) this.select(id);
  }

  onKey(event: KeyboardEvent): void {
    const buttons = this.buttons();
    if (buttons.length === 0) return;
    const active = this.shadowRoot?.activeElement;
    const index = Math.max(0, buttons.findIndex(button => button === active));
    const columns = this.columns(buttons);
    let next: number;
    switch (event.key) {
      case 'ArrowRight': next = index + 1; break;
      case 'ArrowLeft': next = index - 1; break;
      case 'ArrowDown': next = index + columns; break;
      case 'ArrowUp': next = index - columns; break;
      case 'Home': next = 0; break;
      case 'End': next = buttons.length - 1; break;
      case 'Enter':
      case ' ': {
        const id = buttons[index].dataset['id'];
        if (id) this.select(id);
        event.preventDefault();
        return;
      }
      default:
        next = this.typeAhead(event.key, index);
        if (next < 0) return;
    }
    event.preventDefault();
    buttons[Math.min(buttons.length - 1, Math.max(0, next))].focus();
  }

  private buttons(): HTMLButtonElement[] {
    return Array.from(this.shadowRoot?.querySelectorAll<HTMLButtonElement>('.swatch') ?? []);
  }

  /** Columns in the first row, measured from layout so the grid can reflow. */
  private columns(buttons: readonly HTMLButtonElement[]): number {
    const top = buttons[0].offsetTop;
    let count = 0;
    while (count < buttons.length && buttons[count].offsetTop === top) count++;
    return Math.max(1, count);
  }

  private typeAhead(key: string, from: number): number {
    if (key.length !== 1 || key === ' ') return -1;
    const now = performance.now();
    const letter = key.toLowerCase();
    this.typed = now - this.typedAt > TYPEAHEAD_RESET_MS ? letter : this.typed + letter;
    this.typedAt = now;
    const count = this.items.length;
    for (let step = 1; step <= count; step++) {
      const candidate = (from + step) % count;
      if (this.items[candidate].label.toLowerCase().startsWith(this.typed)) return candidate;
    }
    return -1;
  }

  private select(id: string): void {
    if (id === this.selected) return;
    this.selected = id;
    track('swatch.select', { grid: this.label, id });
    this.$emit('select', { id });
  }

  private describe(): void {
    const item = this.items.find(entry => entry.id === this.selected);
    this.hint = item ? item.label + ' is selected' : this.items.length + ' choices';
  }
}

NtpSwatchGrid.define('ntp-swatch-grid');
