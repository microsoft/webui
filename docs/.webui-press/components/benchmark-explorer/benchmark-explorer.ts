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

interface BenchmarkDetailPart {
  label: string;
  value: string;
  wide: boolean;
}

const EMPTY_DATA: BenchmarkData = {
  methodology: "",
  link: "",
  selectedMetric: "noStreamingRequestsPerSecond",
  selectedMetricLabel: "",
  selectedMetricDescription: "",
  hasUnavailable: false,
  metrics: [],
  rows: [],
};

const DETAIL_SEPARATOR = " · ";
const MAD_PREFIX = "MAD ";
const P95_PREFIX = "p95 ";
const SATURATION_PREFIX = "saturation warning: ";

function benchmarkDetailParts(detail: string): BenchmarkDetailPart[] {
  const segments = detail.split(DETAIL_SEPARATOR);
  return segments.map((segment, index) => {
    if (segment.startsWith(MAD_PREFIX)) {
      return {
        label: "Variability (MAD)",
        value: segment.slice(MAD_PREFIX.length),
        wide: false,
      };
    }
    if (segment.startsWith(P95_PREFIX)) {
      return {
        label: "Tail latency (p95)",
        value: segment.slice(P95_PREFIX.length),
        wide: false,
      };
    }
    if (segment.startsWith(SATURATION_PREFIX)) {
      return {
        label: "Saturation warning",
        value: segment.slice(SATURATION_PREFIX.length),
        wide: true,
      };
    }
    if (segment.includes(" B natural")) {
      return { label: "Response size", value: segment, wide: false };
    }
    if (segment.includes(" isolated runs")) {
      return { label: "Samples", value: segment, wide: false };
    }
    if (segments.length >= 5 && index === 0) {
      return { label: "Renderer", value: segment, wide: false };
    }
    if (segments.length >= 5 && index === 1) {
      return { label: "Server", value: segment, wide: false };
    }
    return { label: "Benchmark data", value: segment, wide: false };
  });
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
  @observable benchmarks: BenchmarkData = EMPTY_DATA;

  connectedCallback(): void {
    super.connectedCallback();
    this.enhanceBenchmarkDetails();
  }

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
    this.$flushUpdates();
    this.enhanceBenchmarkDetails();
  }

  private enhanceBenchmarkDetails(): void {
    const profiles = this.shadowRoot?.querySelectorAll<HTMLElement>(
      "[data-benchmark-detail]",
    );
    if (!profiles) return;

    for (const profile of profiles) {
      const detail = profile
        .querySelector(".benchmark-detail-fallback dd")
        ?.textContent
        ?.trim();
      if (!detail) continue;

      for (
        const generated of profile.querySelectorAll(
          ".benchmark-detail-generated",
        )
      ) {
        generated.remove();
      }

      const fragment = document.createDocumentFragment();
      for (const part of benchmarkDetailParts(detail)) {
        const group = document.createElement("div");
        group.className = part.wide
          ? "benchmark-detail-part benchmark-detail-part-wide benchmark-detail-generated"
          : "benchmark-detail-part benchmark-detail-generated";

        const term = document.createElement("dt");
        term.textContent = part.label;
        const description = document.createElement("dd");
        description.textContent = part.value;
        group.append(term, description);
        fragment.append(group);
      }

      profile.append(fragment);
      profile.dataset.enhanced = "true";
    }
  }
}

BenchmarkExplorer.define("benchmark-explorer");
