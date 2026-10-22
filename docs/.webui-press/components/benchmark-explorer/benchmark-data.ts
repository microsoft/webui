// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export const DEFAULT_METRIC = "jsHeapUsedBytes";
export const METRIC_IDS = [
  DEFAULT_METRIC,
  "rendererPrivateBytes",
  "largestContentfulPaintMs",
  "noStreamingRequestsPerSecond",
  "streamingRequestsPerSecond",
] as const;

export type MetricId = (typeof METRIC_IDS)[number];

export interface MetricDefinition {
  id: MetricId;
  label: string;
  unit: "MiB" | "ms" | "req/s";
  rawUnit: "bytes" | "ms" | "req/s";
  direction: "higher" | "lower";
  directionLabel: string;
  description: string;
  methodology: string;
  samples: number;
}

export interface Measurement {
  value: number | null;
  display: string;
  detail: string;
}

export interface BenchmarkCase {
  caseId: string;
  label: string;
  refreshed: boolean;
  refreshLabel: string;
  values: Record<MetricId, Measurement>;
}

export interface BenchmarkSnapshot {
  metrics: MetricDefinition[];
  rows: BenchmarkCase[];
}

export interface BenchmarkRow extends BenchmarkCase {
  available: boolean;
  zero: boolean;
  valueLabel: string;
  detail: string;
  barWidth: number | null;
}

export interface BenchmarkView extends BenchmarkSnapshot {
  metrics: (MetricDefinition & { selected: boolean })[];
  selectedMetric: MetricId;
  activeMetric: MetricDefinition;
  rows: BenchmarkRow[];
  maximumLabel: string;
  midpointLabel: string;
  scaleAvailable: boolean;
  availableCount: number;
  unavailableCount: number;
}

export interface BenchmarkEvidence extends BenchmarkSnapshot {
  schemaVersion: number;
  status: string;
  provenance: {
    publicSource: string;
    sourceAccess: string;
    serverMeasuredRounds: number;
    browserIsolatedRuns: number;
    selectiveRefresh: { refreshedAt: string; caseIds: string[] };
    missing: string[];
    limitations: string[];
  };
}

const DEFINITIONS: Record<MetricId, Omit<MetricDefinition, "id" | "methodology">> = {
  jsHeapUsedBytes: {
    label: "JavaScript heap",
    unit: "MiB",
    rawUnit: "bytes",
    direction: "lower",
    directionLabel: "Lower uses less JS memory",
    description: "Post-readiness JavaScript heap after explicit Chromium garbage collection. This is not total browser memory.",
    samples: 10,
  },
  rendererPrivateBytes: {
    label: "Renderer private bytes",
    unit: "MiB",
    rawUnit: "bytes",
    direction: "lower",
    directionLabel: "Lower uses less renderer memory",
    description: "Private bytes attributed to the isolated Chromium renderer after readiness. This is separate from JavaScript heap.",
    samples: 10,
  },
  largestContentfulPaintMs: {
    label: "Largest contentful paint",
    unit: "ms",
    rawUnit: "ms",
    direction: "lower",
    directionLabel: "Lower paints earlier",
    description: "Pre-interaction LCP from navigation start in an unthrottled lab setup, not field Core Web Vitals.",
    samples: 10,
  },
  noStreamingRequestsPerSecond: {
    label: "Complete response",
    unit: "req/s",
    rawUnit: "req/s",
    direction: "higher",
    directionLabel: "Higher serves more requests",
    description: "Dynamic SSR throughput with a fully collected HTML response. One measured round cannot establish run-to-run variability.",
    samples: 1,
  },
  streamingRequestsPerSecond: {
    label: "Progressive response",
    unit: "req/s",
    rawUnit: "req/s",
    direction: "higher",
    directionLabel: "Higher serves more requests",
    description: "Dynamic SSR throughput with each case's existing response behavior. Some cases remain buffered; see their notes.",
    samples: 1,
  },
};

const numberFormat = new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 });

export function formatMeasurement(value: number | null, metric: MetricDefinition): string {
  if (value === null) return "Unavailable";
  const converted = metric.rawUnit === "bytes" ? value / 1_048_576 : value;
  if (converted > 0 && converted < 0.01) return "<0.01";
  return numberFormat.format(converted);
}

function compareText(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

export function buildBenchmarkView(
  snapshot: BenchmarkSnapshot,
  metricId: MetricId = DEFAULT_METRIC,
): BenchmarkView {
  const metric = snapshot.metrics.find(item => item.id === metricId);
  if (!metric) throw new Error(`Unknown benchmark metric: ${metricId}`);
  let maximum = 0;
  let availableCount = 0;
  for (const row of snapshot.rows) {
    const value = row.values[metricId].value;
    if (value !== null) {
      maximum = Math.max(maximum, value);
      availableCount += 1;
    }
  }
  const rows: BenchmarkRow[] = snapshot.rows.map(row => {
    const active = row.values[metricId];
    return {
      ...row,
      available: active.value !== null,
      zero: active.value === 0,
      valueLabel: active.value === null ? "Unavailable" : `${active.display} ${metric.unit}`,
      detail: active.detail,
      barWidth: active.value === null ? null : maximum === 0 ? 0 : active.value / maximum * 100,
    };
  });
  rows.sort((left, right) => {
    const a = left.values[metricId].value;
    const b = right.values[metricId].value;
    if (a === null && b !== null) return 1;
    if (b === null && a !== null) return -1;
    if (a !== null && b !== null && a !== b) {
      return metric.direction === "higher" ? b - a : a - b;
    }
    return compareText(left.label, right.label) || compareText(left.caseId, right.caseId);
  });
  return {
    metrics: snapshot.metrics.map(item => ({ ...item, selected: item.id === metricId })),
    selectedMetric: metricId,
    activeMetric: metric,
    rows,
    maximumLabel: formatMeasurement(availableCount ? maximum : null, metric),
    midpointLabel: formatMeasurement(availableCount ? maximum / 2 : null, metric),
    scaleAvailable: availableCount > 0,
    availableCount,
    unavailableCount: rows.length - availableCount,
  };
}

export class BenchmarkInputError extends Error {
  readonly code: string;
  readonly field: string;

  constructor(code: string, field: string, message: string) {
    super(`[${code}] ${field}: ${message}\nhelp: Restore the reviewed historical benchmark input, or explicitly review the evidence contract before publishing changed data.`);
    this.name = "BenchmarkInputError";
    this.code = code;
    this.field = field;
  }
}

function invalid(code: string, field: string, message: string): never {
  throw new BenchmarkInputError(code, field, message);
}

function object(value: unknown, field: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    invalid("benchmark-object", field, "Expected an object.");
  }
  return value as Record<string, unknown>;
}

function text(value: unknown, field: string): string {
  if (typeof value !== "string" || value.trim().length === 0 || value.length > 4096) {
    invalid("benchmark-text", field, "Expected nonempty plain text, at most 4096 characters.");
  }
  return value;
}

function array(value: unknown, field: string, count: number): unknown[] {
  if (!Array.isArray(value) || value.length !== count) {
    invalid("benchmark-count", field, `Expected exactly ${count} entries; do not silently exclude results.`);
  }
  return value;
}

function readMetrics(value: unknown): MetricDefinition[] {
  const definitions = new Map<string, Record<string, unknown>>();
  for (const item of array(value, "benchmarks.metrics", METRIC_IDS.length)) {
    const source = object(item, "benchmarks.metrics[]");
    const id = text(source.id, "benchmarks.metrics[].id");
    if (!METRIC_IDS.some(candidate => candidate === id) || definitions.has(id)) {
      invalid("benchmark-metric", id, "Metric identifiers must be known and unique.");
    }
    definitions.set(id, source);
  }
  return METRIC_IDS.map(id => {
    const source = definitions.get(id);
    if (!source) return invalid("benchmark-metric", id, "Missing metric definition.");
    const definition = DEFINITIONS[id];
    if (source.unit !== definition.unit || source.direction !== definition.direction) {
      invalid("benchmark-unit", id, `Expected ${definition.unit} and ${definition.direction} direction.`);
    }
    const methodology = text(source.methodology, `benchmarks.metrics.${id}.methodology`);
    const sampling = definition.samples === 1
      ? "1 × 20 s measured round"
      : "10 randomized cold-cache navigations";
    if (!methodology.includes(sampling)) {
      invalid("benchmark-sampling", id, `The historical contract requires "${sampling}".`);
    }
    return { id, ...definition, methodology };
  });
}

function readMeasurement(value: unknown, metric: MetricDefinition, field: string): Measurement {
  const source = object(value, field);
  if (typeof source.available !== "boolean") {
    invalid("benchmark-availability", field, "An explicit availability flag is required.");
  }
  const measured = source.value;
  if (source.available) {
    if (typeof measured !== "number" || !Number.isFinite(measured) || measured < 0) {
      invalid("benchmark-value", field, "Available measurements must be finite, nonnegative numbers.");
    }
  } else if (measured !== null) {
    invalid("benchmark-unavailable", field, "Unavailable measurements must have an explicit null value, never zero.");
  }
  const detail = text(source.detail, `${field}.detail`);
  if (source.available && metric.samples === 10 && !detail.startsWith("10 isolated runs · ")) {
    invalid("benchmark-sampling", field, "Browser measurement notes must identify the 10 isolated runs.");
  }
  const measuredValue = measured as number | null;
  return { value: measuredValue, display: formatMeasurement(measuredValue, metric), detail };
}

function readRefresh(value: unknown): BenchmarkEvidence["provenance"]["selectiveRefresh"] {
  const source = object(value, "benchmarks.selectiveRefresh");
  const refreshedAt = text(source.refreshedAt, "benchmarks.selectiveRefresh.refreshedAt");
  if (refreshedAt !== "2026-09-04T03:28:11.463Z") {
    invalid("benchmark-refresh", "refreshedAt", "This page documents the selective 2026-09-04 refresh, not a new campaign.");
  }
  const expected = ["webui-bun-serve", "webui-deno-serve", "webui-node-http"];
  const caseIds = array(source.caseIds, "benchmarks.selectiveRefresh.caseIds", 3)
    .map(item => text(item, "benchmarks.selectiveRefresh.caseIds[]"))
    .sort(compareText);
  if (caseIds.some((id, index) => id !== expected[index])) {
    invalid("benchmark-refresh", "caseIds", "The selective refresh must identify the Bun, Deno, and Node WebUI hosts.");
  }
  return { refreshedAt, caseIds };
}

export function prepareBenchmarkEvidence(input: unknown): BenchmarkEvidence {
  const source = object(object(input, "root").benchmarks, "benchmarks");
  const capability = object(source.asyncDataBoundary, "benchmarks.asyncDataBoundary");
  if (capability.canonicalWebuiVersion !== "0.0.28"
    || capability.status !== "available-pending-benchmark-implementation") {
    invalid("benchmark-capability", "benchmarks.asyncDataBoundary", "Expected the historical capability metadata, not a measured-version record.");
  }
  const metrics = readMetrics(source.metrics);
  const refresh = readRefresh(source.selectiveRefresh);
  const ids = new Set<string>();
  const rows = array(source.rows, "benchmarks.rows", 16).map((item, index) => {
    const field = `benchmarks.rows[${index}]`;
    const row = object(item, field);
    const caseId = text(row.caseId, `${field}.caseId`);
    if (ids.has(caseId)) invalid("benchmark-case", field, "Case identifiers must be unique.");
    ids.add(caseId);
    const label = text(row.label, `${field}.label`);
    const sourceValues = object(row.metrics, `${field}.metrics`);
    const values = {} as Record<MetricId, Measurement>;
    for (const metric of metrics) {
      values[metric.id] = readMeasurement(sourceValues[metric.id], metric, `${field}.metrics.${metric.id}`);
    }
    const refreshed = refresh.caseIds.includes(caseId);
    return {
      caseId, label, refreshed,
      refreshLabel: refreshed ? "Refreshed 2026-09-04" : "Earlier result; date not recorded",
      values,
    };
  });
  if (refresh.caseIds.some(id => !ids.has(id))) {
    invalid("benchmark-refresh", "caseIds", "A selectively refreshed case is missing from the results.");
  }
  rows.sort((left, right) => compareText(left.caseId, right.caseId));
  return {
    schemaVersion: 1,
    status: "Historical, limited, mixed-refresh results",
    provenance: {
      publicSource: "https://github.com/microsoft/webui-benchmarks",
      sourceAccess: "The advertised source returned HTTP 404 without authentication during the website audit. Its availability and full contents could not be verified.",
      serverMeasuredRounds: 1,
      browserIsolatedRuns: 10,
      selectiveRefresh: refresh,
      missing: [
        "An anonymously inspectable source revision and raw run artifacts",
        "A complete per-case framework and runtime version trail",
        "Original measurement dates for the other 13 cases",
        "A complete hardware and operating-system record",
        "An unambiguous browser response-mode record",
      ],
      limitations: [
        "One measured server round does not estimate run-to-run variability. Zero MAD is not evidence of confidence or a statistically established winner.",
        "Browser metrics describe 10 isolated runs. Reported MAD and p95 are descriptive, not confidence bounds; the raw samples are not available here.",
        "Only the three listed WebUI host cases were refreshed on 2026-09-04. Other cases retain earlier official results, not a uniform rerun.",
        "WebUI 0.0.28 appears in capability metadata only. It does not establish the version used for every measurement.",
        "Lower memory use is not an overall performance score. JavaScript heap, renderer private bytes, LCP, and server throughput measure different things.",
        "Server throughput uses 64 connections. Saturation warnings in individual notes identify still-rising 128-connection probes; these are not demonstrated maximum throughputs.",
        "The progressive category retains each case's existing behavior, including buffered routes. The dedicated WebUI resumable async data-boundary workload was not measured.",
        "This sanitized extract preserves the five displayed metrics and their notes. It omits bulk server telemetry and unsupported winner summaries; it is not a complete reproducibility package.",
      ],
    },
    metrics,
    rows,
  };
}
