// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { BenchmarkInputError, buildBenchmarkView } from "../benchmark-explorer/benchmark-data.js";
import type {
  BenchmarkRow, BenchmarkSnapshot, MetricDefinition, MetricId,
} from "../benchmark-explorer/benchmark-data.js";

const METRICS = [
  {
    id: "noStreamingRequestsPerSecond",
    label: "RPS",
    caption: "Complete responses",
    description: "Fully collected HTML responses at 64 connections; response sizes differ.",
  },
  {
    id: "streamingRequestsPerSecond",
    label: "Streaming",
    caption: "Streaming responses",
    description: "The progressive category uses 64 connections and each configuration's existing response behavior, including buffered routes. A dedicated WebUI resumable async data-boundary workload was not measured.",
  },
  {
    id: "jsHeapUsedBytes",
    label: "JS heap",
    caption: "JavaScript heap after GC",
    description: "Heap after readiness and explicit Chromium garbage collection, not total browser memory.",
  },
  {
    id: "rendererPrivateBytes",
    label: "Renderer memory",
    caption: "Renderer private bytes",
    description: "Private bytes of the isolated Chromium renderer after readiness, not server RSS. Some samples were unavailable; see the full data for available sample counts.",
  },
] as const satisfies readonly { id: MetricId; label: string; caption: string; description: string }[];

const CASE_IDS = ["webui-rust-actix", "react", "next"];

export interface BenchmarkPreview {
  id: (typeof METRICS)[number]["id"];
  label: string;
  caption: string;
  directionLabel: string;
  noteNumber: number;
  selected: boolean;
  tabIndex: 0 | -1;
  unit: MetricDefinition["unit"];
  maximumLabel: string;
  midpointLabel: string;
  scaleAvailable: boolean;
  rows: Pick<BenchmarkRow, "caseId" | "label" | "valueLabel" | "barWidth" | "available" | "zero">[];
}

export interface BenchmarkNote {
  id: BenchmarkPreview["id"];
  label: string;
  description: string;
  samplingLabel: string;
}

export function buildBenchmarkPreviews(snapshot: BenchmarkSnapshot): BenchmarkPreview[] {
  return METRICS.map((presentation, index): BenchmarkPreview => {
    const view = buildBenchmarkView(snapshot, presentation.id);
    const rows = CASE_IDS.map(caseId => {
      const row = view.rows.find(candidate => candidate.caseId === caseId);
      if (!row) {
        throw new BenchmarkInputError("benchmark-preview", caseId,
          "Missing a required preview configuration; do not silently substitute another row.");
      }
      return {
        caseId: row.caseId,
        label: row.caseId === "webui-rust-actix" ? "WebUI" : row.label,
        valueLabel: row.valueLabel,
        barWidth: row.barWidth,
        available: row.available,
        zero: row.zero,
      };
    });
    return {
      id: presentation.id,
      label: presentation.label,
      caption: presentation.caption,
      directionLabel: view.activeMetric.direction === "higher" ? "Higher is better" : "Lower is better",
      noteNumber: index + 1,
      selected: index === 0,
      tabIndex: index === 0 ? 0 : -1,
      unit: view.activeMetric.unit,
      maximumLabel: view.maximumLabel,
      midpointLabel: view.midpointLabel,
      scaleAvailable: view.scaleAvailable,
      rows,
    };
  });
}

export function buildBenchmarkNotes(snapshot: BenchmarkSnapshot): BenchmarkNote[] {
  return METRICS.map(presentation => {
    const metric = snapshot.metrics.find(candidate => candidate.id === presentation.id);
    if (!metric) {
      throw new BenchmarkInputError("benchmark-preview", presentation.id,
        "Missing a required metric definition; restore the benchmark snapshot.");
    }
    return {
      id: presentation.id,
      label: presentation.caption,
      description: presentation.description,
      samplingLabel: metric.unit === "req/s"
        ? `${metric.samples} measured round (20 s); variability is not established.`
        : `${metric.samples} isolated Chromium desktop runs.`,
    };
  });
}
