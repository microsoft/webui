// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { build } from "esbuild";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const helper = new URL("../components/benchmark-explorer/benchmark-data.ts", import.meta.url);
const previewHelper = new URL("../components/benchmark-preview/benchmark-preview-data.ts", import.meta.url);
const compiled = new URL("../generated/benchmark-data.mjs", import.meta.url);
const compiledPreview = new URL("../generated/benchmark-preview-data.mjs", import.meta.url);
const source = new URL("../state/benchmark-summary.json", import.meta.url);
const state = new URL("../generated/benchmark-state.json", import.meta.url);
const homeState = new URL("../generated/home-benchmark-state.json", import.meta.url);
const download = new URL("../public/benchmarks/official-results.json", import.meta.url);

try {
  try {
    await build({
      absWorkingDir: fileURLToPath(new URL("../", import.meta.url)),
      entryPoints: [fileURLToPath(helper), fileURLToPath(previewHelper)],
      outdir: fileURLToPath(new URL(".", compiled)),
      entryNames: "[name]",
      outExtension: { ".js": ".mjs" },
      bundle: true,
      platform: "node",
      format: "esm",
      target: "node22",
      logLevel: "silent",
    });
  } catch (error) {
    throw new Error(`[benchmark-compile] Cannot compile the benchmark data helpers.\nhelp: Fix ${fileURLToPath(helper)} and ${fileURLToPath(previewHelper)}, and ensure the generated docs directory is writable.`, { cause: error });
  }
  const { buildBenchmarkView, prepareBenchmarkEvidence } = await import(compiled.href);
  const { buildBenchmarkPreviews } = await import(compiledPreview.href);
  let input;
  try {
    input = JSON.parse(await readFile(source, "utf8"));
  } catch (error) {
    throw new Error(`[benchmark-read] Cannot read valid JSON from ${fileURLToPath(source)}.\nhelp: Restore the synchronized official campaign source file before building the docs.`, { cause: error });
  }
  const evidence = prepareBenchmarkEvidence(input);
  const fullView = buildBenchmarkView(evidence);
  const renderedState = { benchmarks: fullView };
  const previewMetrics = buildBenchmarkPreviews(evidence);
  const previewState = {
    previewMetrics,
    previewCaseCount: previewMetrics[0].rows.length,
    previewTotalCases: evidence.rows.length,
  };
  for (const [file, content] of [[download, evidence], [state, renderedState], [homeState, previewState]]) {
    try {
      await mkdir(new URL(".", file), { recursive: true });
      await writeFile(file, `${JSON.stringify(content)}\n`, "utf8");
    } catch (error) {
      throw new Error(`[benchmark-write] Cannot write ${fileURLToPath(file)}.\nhelp: Make the generated docs directories writable, then rerun the build.`, { cause: error });
    }
  }
  console.log(`Prepared official benchmark state and sanitized JSON (${evidence.rows.length} cases, ${evidence.metrics.length} metrics).`);
} catch (error) {
  console.error(error instanceof Error ? error.message : "[benchmark-prepare] Unknown evidence preparation failure.");
  process.exitCode = 1;
}
