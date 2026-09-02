// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";

interface MetricDefinition {
  id: string;
  label: string;
  unit: string;
  direction: "higher" | "lower";
  directionLabel: string;
  description: string;
  methodology: string;
  available: boolean;
  selected: boolean;
}

interface MetricValue {
  available: boolean;
  value: number | null;
  valueDisplay: string;
  detail: string;
}

interface BenchmarkRow {
  caseId: string;
  label: string;
  primary: boolean;
  rank: string;
  activeAvailable: boolean;
  activeDetail: string;
  activeValueDisplay: string;
  activeUnit: string;
  activeWidth: string;
  metrics: Record<string, MetricValue>;
}

interface BenchmarkData {
  methodology: string;
  link: string;
  selectedMetric: string;
  selectedMetricLabel: string;
  selectedMetricDescription: string;
  hasUnavailable: boolean;
  metrics: MetricDefinition[];
  rows: BenchmarkRow[];
}

const EMPTY_DATA: BenchmarkData = {
  methodology: "",
  link: "",
  selectedMetric: "requestsPerSecond",
  selectedMetricLabel: "",
  selectedMetricDescription: "",
  hasUnavailable: false,
  metrics: [],
  rows: [],
};

function metricValue(row: BenchmarkRow, metricId: string): MetricValue {
  return row.metrics[metricId] ?? {
    available: false,
    value: null,
    valueDisplay: "Unavailable",
    detail: "No supported measurement",
  };
}

function rankedRows(
  rows: BenchmarkRow[],
  metric: MetricDefinition,
): BenchmarkRow[] {
  const ranked = rows.map((row) => ({ ...row }));
  ranked.sort((left, right) => {
    const leftMetric = metricValue(left, metric.id);
    const rightMetric = metricValue(right, metric.id);
    if (leftMetric.available !== rightMetric.available) {
      return leftMetric.available ? -1 : 1;
    }
    if (!leftMetric.available || !rightMetric.available) {
      return left.label.localeCompare(right.label);
    }
    const leftValue = leftMetric.value ?? 0;
    const rightValue = rightMetric.value ?? 0;
    const difference = metric.direction === "higher"
      ? rightValue - leftValue
      : leftValue - rightValue;
    return difference === 0
      ? left.label.localeCompare(right.label)
      : difference;
  });

  const availableValues = ranked
    .map((row) => metricValue(row, metric.id))
    .filter((value) => value.available && value.value !== null)
    .map((value) => value.value as number);
  const maximum = availableValues.length === 0
    ? 0
    : Math.max(...availableValues);

  let previousDisplay = "";
  let displayedRank = 0;
  return ranked.map((row, index) => {
    const active = metricValue(row, metric.id);
    if (active.valueDisplay !== previousDisplay) {
      displayedRank = index + 1;
      previousDisplay = active.valueDisplay;
    }
    let width = 0;
    if (active.available && active.value !== null && active.value > 0) {
      width = active.value / maximum * 100;
    }
    return {
      ...row,
      rank: String(displayedRank),
      activeAvailable: active.available,
      activeDetail: active.detail,
      activeValueDisplay: active.valueDisplay,
      activeUnit: active.available ? metric.unit : "",
      activeWidth: `${width.toFixed(1)}%`,
    };
  });
}

export class BenchmarkExplorer extends WebUIElement {
  @observable benchmarks: BenchmarkData = EMPTY_DATA;

  selectMetric(event: Event): void {
    const target = event.currentTarget;
    if (!(target instanceof HTMLButtonElement)) return;
    const metricId = target.dataset.metric;
    if (!metricId || metricId === this.benchmarks.selectedMetric) return;
    const metric = this.benchmarks.metrics.find((candidate) =>
      candidate.id === metricId && candidate.available
    );
    if (!metric) return;

    const metrics = this.benchmarks.metrics.map((candidate) => ({
      ...candidate,
      selected: candidate.id === metricId,
    }));
    const rows = rankedRows(this.benchmarks.rows, metric);
    this.benchmarks = {
      ...this.benchmarks,
      methodology: metric.methodology,
      selectedMetric: metric.id,
      selectedMetricLabel: metric.label,
      selectedMetricDescription: metric.description,
      hasUnavailable: rows.some((row) =>
        !metricValue(row, metric.id).available
      ),
      metrics,
      rows,
    };
  }
}

BenchmarkExplorer.define("benchmark-explorer");
