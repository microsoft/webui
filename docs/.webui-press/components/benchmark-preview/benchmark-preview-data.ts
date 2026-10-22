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
    description: "100-todo app. Complete responses; response sizes differ.",
  },
  {
    id: "streamingRequestsPerSecond",
    label: "Streaming",
    description: "100-todo app. Progressive responses; streaming behavior differs.",
  },
  {
    id: "jsHeapUsedBytes",
    label: "JS heap",
    description: "100-todo app. Heap after GC, not total browser memory.",
  },
  {
    id: "rendererPrivateBytes",
    label: "Renderer memory",
    description: "100-todo app. Renderer private bytes, not RSS.",
  },
] as const satisfies readonly { id: MetricId; label: string; description: string }[];

const CASE_IDS = ["webui-rust-actix", "react", "next"];

export interface BenchmarkPreview {
  id: (typeof METRICS)[number]["id"];
  label: string;
  description: string;
  directionLabel: string;
  samplingLabel: string;
  selected: boolean;
  tabIndex: 0 | -1;
  unit: MetricDefinition["unit"];
  samples: number;
  maximumLabel: string;
  midpointLabel: string;
  scaleAvailable: boolean;
  rows: Pick<BenchmarkRow, "caseId" | "label" | "valueLabel" | "barWidth" | "available" | "zero">[];
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
        label: caseId === "webui-rust-actix" ? "WebUI" : row.label,
        valueLabel: row.valueLabel,
        barWidth: row.barWidth,
        available: row.available,
        zero: row.zero,
      };
    });
    const { unit, samples, directionLabel } = view.activeMetric;
    return {
      ...presentation,
      directionLabel,
      samplingLabel: unit === "req/s"
        ? `${samples} measured round (20 s); variability is not established.`
        : `${samples} isolated Chromium desktop runs.`,
      selected: index === 0,
      tabIndex: index === 0 ? 0 : -1,
      unit,
      samples,
      maximumLabel: view.maximumLabel,
      midpointLabel: view.midpointLabel,
      scaleAvailable: view.scaleAvailable,
      rows,
    };
  });
}
