// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";
import { buildBenchmarkView } from "./benchmark-data.js";
import type { BenchmarkView, MetricId } from "./benchmark-data.js";

export class BenchmarkExplorer extends WebUIElement {
  @observable benchmarks!: BenchmarkView;
  @observable ready = false;

  protected hydratedCallback(): void {
    this.ready = true;
  }

  selectMetric(metricId: MetricId): void {
    if (metricId !== this.benchmarks.selectedMetric) {
      this.benchmarks = buildBenchmarkView(this.benchmarks, metricId);
    }
  }

  selectMetricFromInput(event: Event): void {
    const select = event.currentTarget;
    if (!(select instanceof HTMLSelectElement)) return;
    const metric = this.benchmarks.metrics.find(item => item.id === select.value);
    if (metric) this.selectMetric(metric.id);
  }
}

BenchmarkExplorer.define("benchmark-explorer");
