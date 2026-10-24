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

const CASE_IDS = [
  "astro", "lit", "next", "nuxt", "preact", "react", "react-tanstack",
  "solid", "sveltekit", "vue", "webui-bun-serve", "webui-fast-rust-actix",
  "webui-node-http", "webui-rust-actix", "webui-shadowdom-rust-actix",
];

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
    campaign: {
      canonicalWebuiVersion: string;
      responseModes: ["complete", "progressive"];
      uniformOfficialMatrix: true;
      sourceCommit: string;
      sourceDigest: string;
      fixtureDigest: string;
      evidenceDigest: string;
      matrixId: string;
      status: "official-uniform";
      progressiveMeasuredAt: string;
      completeMeasuredAt: string;
      browserMeasuredAt: string;
    };
    missing: string[];
    limitations: string[];
  };
}

// Prepared evidence has canonical case/metric order. Exclude the evidence digest itself.
export function prepareBenchmarkCampaignDigestPayload(evidence: BenchmarkEvidence) {
  const campaign = evidence.provenance.campaign;
  return {
    campaign: {
      canonicalWebuiVersion: campaign.canonicalWebuiVersion,
      responseModes: campaign.responseModes,
      uniformOfficialMatrix: campaign.uniformOfficialMatrix,
      matrixId: campaign.matrixId,
      status: campaign.status,
      sourceCommit: campaign.sourceCommit,
      sourceDigest: campaign.sourceDigest,
      fixtureDigest: campaign.fixtureDigest,
      serverMeasuredRounds: evidence.provenance.serverMeasuredRounds,
      browserIsolatedRuns: evidence.provenance.browserIsolatedRuns,
      rowCount: evidence.rows.length,
      workingTreeDirty: false,
      progressiveMeasuredAt: campaign.progressiveMeasuredAt,
      completeMeasuredAt: campaign.completeMeasuredAt,
      browserMeasuredAt: campaign.browserMeasuredAt,
    },
    metrics: evidence.metrics.map(metric => ({
      id: metric.id,
      label: metric.label,
      unit: metric.unit,
      rawUnit: metric.rawUnit,
      direction: metric.direction,
      directionLabel: metric.directionLabel,
      description: metric.description,
      methodology: metric.methodology,
      samples: metric.samples,
    })),
    rows: evidence.rows.map(row => ({
      caseId: row.caseId,
      label: row.label,
      metrics: Object.fromEntries(METRIC_IDS.map(id => {
        const measurement = row.values[id];
        return [id, {
          available: measurement.value !== null,
          value: measurement.value,
          detail: measurement.detail,
        }];
      })),
    })),
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
    super(`[${code}] ${field}: ${message}\nhelp: Restore the reviewed official campaign input, or explicitly review the evidence contract before publishing changed data.`);
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
      invalid("benchmark-sampling", id, `The official campaign contract requires "${sampling}".`);
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
  if (source.available && metric.samples === 10) {
    const sampleCount = /^([1-9]|10) isolated runs · /u.exec(detail);
    const available = sampleCount ? Number(sampleCount[1]) : 0;
    const unavailable = / · ([1-9]) unavailable$/u.exec(detail);
    const missing = unavailable ? Number(unavailable[1]) : 0;
    if (!available || available + missing !== metric.samples) {
      invalid("benchmark-sampling", field, "Browser notes must account for all 10 runs, including unavailable samples.");
    }
  }
  const measuredValue = measured as number | null;
  return { value: measuredValue, display: formatMeasurement(measuredValue, metric), detail };
}

export function prepareBenchmarkEvidence(input: unknown): BenchmarkEvidence {
  const source = object(object(input, "root").benchmarks, "benchmarks");
  const capability = object(source.asyncDataBoundary, "benchmarks.asyncDataBoundary");
  const campaign = object(source.campaign, "benchmarks.campaign");
  const canonicalWebuiVersion = text(
    campaign.canonicalWebuiVersion,
    "benchmarks.campaign.canonicalWebuiVersion",
  );
  if (!/^\d+\.\d+\.\d+$/u.test(canonicalWebuiVersion)
    || capability.canonicalWebuiVersion !== canonicalWebuiVersion
    || capability.status !== "available-pending-benchmark-implementation") {
    invalid("benchmark-capability", "benchmarks.asyncDataBoundary", "Expected campaign capability metadata for a canonical WebUI release.");
  }
  const responseModes = array(campaign.responseModes, "benchmarks.campaign.responseModes", 2);
  if (responseModes[0] !== "complete" || responseModes[1] !== "progressive") {
    invalid("benchmark-campaign", "benchmarks.campaign.responseModes", "Expected the complete and progressive official suites in order.");
  }
  if (campaign.status !== "official-uniform"
    || campaign.uniformOfficialMatrix !== true
    || campaign.rowCount !== 15
    || campaign.matrixId !== "official-ssr-matrix"
    || campaign.workingTreeDirty !== false
    || campaign.serverMeasuredRounds !== 1
    || campaign.browserRoundsPerApp !== 10
    || source.responseMode !== "comparison"
    || !Array.isArray(source.responseModes)
    || source.responseModes.join(",") !== responseModes.join(",")) {
    invalid("benchmark-campaign", "benchmarks.campaign", "Expected a uniform 15-row official campaign.");
  }
  const sourceCommit = text(campaign.sourceCommit, "benchmarks.campaign.sourceCommit");
  const sourceDigest = text(campaign.sourceDigest, "benchmarks.campaign.sourceDigest");
  const fixtureDigest = text(campaign.fixtureDigest, "benchmarks.campaign.fixtureDigest");
  const evidenceDigest = text(campaign.evidenceDigest, "benchmarks.campaign.evidenceDigest");
  if (!/^[0-9a-f]{40}$/u.test(sourceCommit)
    || !/^[0-9a-f]{64}$/u.test(sourceDigest)
    || !/^[0-9a-f]{64}$/u.test(evidenceDigest)
    || sourceDigest !== fixtureDigest) {
    invalid("benchmark-campaign", "benchmarks.campaign", "Expected a source commit, matching source/fixture SHA-256 digests, and a separate evidence SHA-256 digest.");
  }
  const progressiveMeasuredAt = text(campaign.progressiveMeasuredAt, "benchmarks.campaign.progressiveMeasuredAt");
  const completeMeasuredAt = text(campaign.completeMeasuredAt, "benchmarks.campaign.completeMeasuredAt");
  const browserMeasuredAt = text(campaign.browserMeasuredAt, "benchmarks.campaign.browserMeasuredAt");
  if (![progressiveMeasuredAt, completeMeasuredAt, browserMeasuredAt]
    .every(value => Number.isFinite(Date.parse(value)))) {
    invalid("benchmark-campaign", "benchmarks.campaign", "Expected report timestamps for both SSR suites and the browser suite.");
  }
  const metrics = readMetrics(source.metrics);
  const ids = new Set<string>();
  const rows = array(source.rows, "benchmarks.rows", 15).map((item, index) => {
    const field = `benchmarks.rows[${index}]`;
    const row = object(item, field);
    const caseId = text(row.caseId, `${field}.caseId`);
    if (!CASE_IDS.includes(caseId)) invalid("benchmark-case", field, "Unknown official case identifier.");
    if (ids.has(caseId)) invalid("benchmark-case", field, "Case identifiers must be unique.");
    ids.add(caseId);
    const label = text(row.label, `${field}.label`);
    const sourceValues = object(row.metrics, `${field}.metrics`);
    const values = {} as Record<MetricId, Measurement>;
    for (const metric of metrics) {
      values[metric.id] = readMeasurement(sourceValues[metric.id], metric, `${field}.metrics.${metric.id}`);
    }
    return {
      caseId,
      label,
      values,
    };
  });
  rows.sort((left, right) => compareText(left.caseId, right.caseId));
  return {
    schemaVersion: 1,
    status: "Official uniform campaign",
    provenance: {
      publicSource: "https://github.com/microsoft/webui-benchmarks",
      sourceAccess: "Exported from the official reports in webui-benchmarks. This download is a sanitized extract, not a raw-run archive.",
      serverMeasuredRounds: 1,
      browserIsolatedRuns: 10,
      campaign: {
        canonicalWebuiVersion,
        responseModes: ["complete", "progressive"],
        uniformOfficialMatrix: true,
        sourceCommit,
        sourceDigest,
        fixtureDigest,
        evidenceDigest,
        matrixId: "official-ssr-matrix",
        status: "official-uniform",
        progressiveMeasuredAt,
        completeMeasuredAt,
        browserMeasuredAt,
      },
      missing: [
        "A public cross-framework raw-run archive is not embedded in this sanitized docs extract.",
      ],
      limitations: [
        "One measured server round does not estimate run-to-run variability. Zero MAD is not evidence of confidence or a statistically established winner.",
        "Browser metrics describe 10 isolated runs. Reported MAD and p95 are descriptive, not confidence bounds; the raw samples are not available here.",
        "Some renderer-private measurements are unavailable. Each row's notes disclose the available and unavailable sample counts; these medians do not all use 10 samples.",
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
