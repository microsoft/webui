// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";
import type { BenchmarkPreview } from "./benchmark-preview-data.js";

export class BenchmarkPreviewElement extends WebUIElement {
  @observable previewMetrics: BenchmarkPreview[] = [];
  @observable previewReady = false;

  protected hydratedCallback(): void {
    this.previewReady = true;
  }

  selectMetric(id: BenchmarkPreview["id"]): void {
    const selected = this.previewMetrics.find(metric => metric.id === id);
    if (!selected) {
      throw new Error(`[WebUI docs] Unknown benchmark metric "${id}". Reload the page or open the full benchmark results.`);
    }
    if (selected.selected) return;
    this.previewMetrics = this.previewMetrics.map((metric): BenchmarkPreview => ({
      ...metric,
      selected: metric.id === id,
      tabIndex: metric.id === id ? 0 : -1,
    }));
  }

  onMetricKeyDown(event: KeyboardEvent): void {
    const current = this.previewMetrics.findIndex(metric => metric.selected);
    const count = this.previewMetrics.length;
    let next: number;
    switch (event.key) {
      case "ArrowRight": next = (current + 1) % count; break;
      case "ArrowLeft": next = (current + count - 1) % count; break;
      case "Home": next = 0; break;
      case "End": next = count - 1; break;
      default: return;
    }
    event.preventDefault();
    const metric = this.previewMetrics[next];
    if (current < 0 || !metric) {
      throw new Error("[WebUI docs] Benchmark selection is unavailable. Reload the page or open the full results.");
    }
    this.selectMetric(metric.id);
    const tab = this.shadowRoot?.getElementById(`preview-tab-${metric.id}`);
    if (!tab) {
      throw new Error("[WebUI docs] Cannot focus the selected benchmark tab. Reload the page.");
    }
    tab.focus();
  }
}

BenchmarkPreviewElement.define("benchmark-preview");
