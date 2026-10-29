// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '@microsoft/webui-framework';

/**
 * A miniature reading list drawn with the draft layout.
 *
 * Spacing and highlighting are CSS on the host's `density` and `highlight`
 * attributes. The script measures how many rows fit in the preview, so the
 * caption follows every change of spacing. Only the settings dialog's lazy
 * body uses this component, so its code loads with that body.
 */
export class LayoutPreview extends WebUIElement {
  /** Rows fully visible in the preview at the current spacing. */
  @observable visibleRows = 0;

  viewport!: HTMLElement;
  rows!: HTMLElement;
  private observer?: ResizeObserver;

  override connectedCallback(): void {
    super.connectedCallback();
    if (!this.viewport || !this.rows) return;
    // The viewport gets its size when the dialog opens; the rows resize when the spacing changes.
    this.observer ??= new ResizeObserver(() => this.measure());
    this.observer.observe(this.viewport);
    this.observer.observe(this.rows);
  }

  override disconnectedCallback(): void {
    this.observer?.disconnect();
    super.disconnectedCallback();
  }

  private measure(): void {
    const height = this.viewport.clientHeight;
    let visible = 0;
    // A closed dialog has no layout, so nothing is visible.
    if (height > 0) {
      for (const row of this.rows.children) {
        const { offsetTop, offsetHeight } = row as HTMLElement;
        if (offsetTop + offsetHeight > height) break;
        visible++;
      }
    }
    this.visibleRows = visible;
  }
}

LayoutPreview.define('layout-preview');
