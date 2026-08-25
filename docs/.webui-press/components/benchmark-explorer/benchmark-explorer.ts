// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from "@microsoft/webui-framework";

interface MetricDefinition {
  id: string;
  label: string;
  unit: string;
  direction: "higher" | "lower";
  directionLabel: string;
  description: string;
  methodology: string;
  summary: string;
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
  title: string;
  description: string;
  methodology: string;
  link: string;
  selectedMetric: string;
  selectedMetricLabel: string;
  selectedMetricDescription: string;
  metricSummary: string;
  hasUnavailable: boolean;
  metrics: MetricDefinition[];
  rows: BenchmarkRow[];
}

const EMPTY_DATA: BenchmarkData = {
  title: "",
  description: "",
  methodology: "",
  link: "",
  selectedMetric: "requestsPerSecond",
  selectedMetricLabel: "",
  selectedMetricDescription: "",
  metricSummary: "",
  hasUnavailable: false,
  metrics: [],
  rows: [],
};

function isBenchmarkData(value: unknown): value is BenchmarkData {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<BenchmarkData>;
  return typeof candidate.title === "string"
    && typeof candidate.selectedMetric === "string"
    && Array.isArray(candidate.metrics)
    && Array.isArray(candidate.rows);
}

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
  @attr({ attribute: "data-json" }) dataJson = "";
  @observable data: BenchmarkData = EMPTY_DATA;

  connectedCallback(): void {
    super.connectedCallback();
    if (this.data.metrics.length > 0 || this.dataJson.length === 0) return;
    const parsed: unknown = JSON.parse(this.dataJson);
    if (!isBenchmarkData(parsed)) {
      throw new TypeError(
        "benchmark-explorer data-json must contain generated benchmark explorer state",
      );
    }
    this.data = parsed;
  }

  selectMetric(event: Event): void {
    const target = event.currentTarget;
    if (!(target instanceof HTMLButtonElement)) return;
    const metricId = target.dataset.metric;
    if (!metricId || metricId === this.data.selectedMetric) return;
    const metric = this.data.metrics.find((candidate) =>
      candidate.id === metricId && candidate.available
    );
    if (!metric) return;

    const metrics = this.data.metrics.map((candidate) => ({
      ...candidate,
      selected: candidate.id === metricId,
    }));
    const rows = rankedRows(this.data.rows, metric);
    this.data = {
      ...this.data,
      methodology: metric.methodology,
      selectedMetric: metric.id,
      selectedMetricLabel: metric.label,
      selectedMetricDescription: metric.description,
      metricSummary: metric.summary,
      hasUnavailable: rows.some((row) =>
        !metricValue(row, metric.id).available
      ),
      metrics,
      rows,
    };
  }
}

BenchmarkExplorer.define("benchmark-explorer");
