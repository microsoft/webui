// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Runtime benchmark for the WebUI Node.js addon.
 *
 * This intentionally runs from Node rather than Criterion so every measured
 * operation crosses the real V8/N-API boundary used by applications.
 */

import assert from "node:assert/strict";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// ── Configuration ─────────────────────────────────────────────────────

const __dirname = dirname(fileURLToPath(import.meta.url));
const CONTACT_COUNTS = [10, 100, 1000];
const RENDER_OPTIONS = Object.freeze({ requestPath: "/contacts" });
const SNAPSHOT_SCHEMA = 1;
const BASELINE_NAME_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;

// ── CLI parsing ───────────────────────────────────────────────────────

function usage() {
  console.log(`Usage: node bench.mjs [options]

Options:
  --quick        Run a short smoke benchmark
  --json         Emit the raw report as JSON
  --allow-debug  Permit a debug addon build (not representative)
  --help         Show this help

Baseline environment variables:
  WEBUI_BENCH_SAVE=NAME     Save target/bench-baselines/node-addon-NAME.json
  WEBUI_BENCH_COMPARE=NAME  Compare P50 and output shape with a saved baseline

Build prerequisites:
  cargo build --release -p microsoft-webui-node
  pnpm --filter @microsoft/webui build`);
}

function parseArgs(argv) {
  const known = new Set(["--quick", "--json", "--allow-debug", "--help"]);
  for (const arg of argv) {
    if (!known.has(arg)) {
      throw new Error(`Unknown option: ${arg}`);
    }
  }
  return {
    quick: argv.includes("--quick") || process.env.WEBUI_BENCH_QUICK === "1",
    json: argv.includes("--json"),
    allowDebug: argv.includes("--allow-debug"),
    help: argv.includes("--help"),
    saveBaseline: process.env.WEBUI_BENCH_SAVE?.trim() || null,
    compareBaseline: process.env.WEBUI_BENCH_COMPARE?.trim() || null,
  };
}

function validateOptions(options) {
  for (const [variable, name] of [
    ["WEBUI_BENCH_SAVE", options.saveBaseline],
    ["WEBUI_BENCH_COMPARE", options.compareBaseline],
  ]) {
    if (name !== null && !BASELINE_NAME_PATTERN.test(name)) {
      throw new Error(
        `${variable} must be 1-64 characters using only letters, numbers, ` +
          "dot, underscore, or hyphen",
      );
    }
  }
  if (options.saveBaseline && options.compareBaseline) {
    throw new Error(
      "WEBUI_BENCH_SAVE and WEBUI_BENCH_COMPARE cannot be used together",
    );
  }
  if (options.quick && (options.saveBaseline || options.compareBaseline)) {
    throw new Error("quick smoke results cannot be saved or compared as baselines");
  }
  if (options.json && (options.saveBaseline || options.compareBaseline)) {
    throw new Error("--json cannot be combined with baseline save or compare mode");
  }
}

// ── Contact Book fixture ──────────────────────────────────────────────

function buildState(seedState, contactCount) {
  assert.ok(
    Array.isArray(seedState.contacts) && seedState.contacts.length > 0,
    "contact-book state fixture has no contacts",
  );
  assert.ok(
    Array.isArray(seedState.groups),
    "contact-book state fixture has no groups",
  );
  const contacts = Array.from({ length: contactCount }, (_, index) => ({
    ...seedState.contacts[index % seedState.contacts.length],
    id: String(index + 1),
  }));
  const favoriteContacts = contacts.filter((contact) => contact.favorite);
  return {
    ...seedState,
    page: "contacts",
    totalContacts: contactCount,
    totalFavorites: favoriteContacts.length,
    totalGroups: seedState.groups.length,
    contacts,
    filteredContacts: contacts,
    recentContacts: contacts.slice(-5),
    favoriteContacts,
    selectedContact: null,
  };
}

// ── Sampling and statistics ───────────────────────────────────────────

function percentile(sortedSamples, quantile) {
  const index = Math.min(
    sortedSamples.length - 1,
    Math.max(0, Math.ceil(sortedSamples.length * quantile) - 1),
  );
  return sortedSamples[index];
}

function summarize(samples) {
  assert.ok(samples.length > 0, "benchmark must collect at least one sample");
  const sorted = [...samples].sort((left, right) => left - right);
  const meanNs = samples.reduce((total, sample) => total + sample, 0) / samples.length;
  return {
    samples: samples.length,
    minNs: sorted[0],
    p50Ns: percentile(sorted, 0.5),
    p95Ns: percentile(sorted, 0.95),
    p99Ns: percentile(sorted, 0.99),
    maxNs: sorted[sorted.length - 1],
    meanNs,
  };
}

function shouldContinue(sampleCount, elapsedNs, config) {
  return (
    sampleCount < config.minSamples ||
    (sampleCount < config.maxSamples && elapsedNs < config.targetDurationNs)
  );
}

function collectSyncSamples(operation, config) {
  let lastValue;
  for (let index = 0; index < config.warmupIterations; index += 1) {
    operation();
  }

  if (typeof global.gc === "function") {
    global.gc();
  }

  const samples = [];
  let elapsedNs = 0;
  do {
    const start = process.hrtime.bigint();
    lastValue = operation();
    const durationNs = Number(process.hrtime.bigint() - start);
    samples.push(durationNs);
    elapsedNs += durationNs;
  } while (shouldContinue(samples.length, elapsedNs, config));

  // Keep the operation's result observably live through the measurement loop.
  if (lastValue === undefined) {
    throw new Error("benchmark operation unexpectedly returned undefined");
  }

  return summarize(samples);
}

function collectStreamSamples(protocol, stateJson, expectedCodeUnits, config) {
  const invoke = (recordTiming) => {
    let chunkCount = 0;
    let codeUnits = 0;
    let firstCallbackNs = null;
    const start = process.hrtime.bigint();

    protocol.renderStream(
      stateJson,
      (chunk) => {
        if (recordTiming && firstCallbackNs === null) {
          firstCallbackNs = Number(process.hrtime.bigint() - start);
        }
        chunkCount += 1;
        codeUnits += chunk.length;
      },
      RENDER_OPTIONS,
    );

    const totalNs = Number(process.hrtime.bigint() - start);
    assert.equal(codeUnits, expectedCodeUnits, "streamed output length changed");
    assert.ok(chunkCount > 0, "renderStream emitted no chunks");
    return { chunkCount, firstCallbackNs, totalNs };
  };

  for (let index = 0; index < config.warmupIterations; index += 1) {
    invoke(false);
  }

  if (typeof global.gc === "function") {
    global.gc();
  }

  const firstCallbackSamples = [];
  const totalSamples = [];
  let elapsedNs = 0;
  let chunkCount = 0;
  do {
    const sample = invoke(true);
    assert.notEqual(sample.firstCallbackNs, null);
    firstCallbackSamples.push(sample.firstCallbackNs);
    totalSamples.push(sample.totalNs);
    elapsedNs += sample.totalNs;
    chunkCount = sample.chunkCount;
  } while (shouldContinue(totalSamples.length, elapsedNs, config));

  return {
    chunkCount,
    firstCallback: summarize(firstCallbackSamples),
    total: summarize(totalSamples),
  };
}

function makeResult(name, metric, latency, details = {}) {
  return {
    name,
    metric,
    ...details,
    latency,
  };
}

// ── Baseline snapshots ────────────────────────────────────────────────

function snapshotPath(name) {
  return resolve(
    __dirname,
    "..",
    "..",
    "..",
    "target",
    "bench-baselines",
    `node-addon-${name}.json`,
  );
}

function snapshotRows(report) {
  return report.results.map((result) => ({
    name: result.name,
    metric: result.metric,
    contacts: result.contacts ?? null,
    inputBytes: result.inputBytes ?? null,
    outputBytes: result.outputBytes ?? null,
    chunkCount: result.chunkCount ?? null,
    samples: result.latency.samples,
    p50Ns: result.latency.p50Ns,
    p95Ns: result.latency.p95Ns,
    p99Ns: result.latency.p99Ns,
    meanNs: result.latency.meanNs,
  }));
}

function saveSnapshot(name, report) {
  const path = snapshotPath(name);
  mkdirSync(dirname(path), { recursive: true });
  const snapshot = {
    schema: SNAPSHOT_SCHEMA,
    name,
    timestampUnix: Math.floor(Date.now() / 1000),
    environment: {
      node: report.environment.node,
      napi: report.environment.napi,
      platform: report.environment.platform,
      arch: report.environment.arch,
    },
    fixture: report.fixture,
    rows: snapshotRows(report),
  };
  writeFileSync(path, `${JSON.stringify(snapshot, null, 2)}\n`);
  console.log(`\n✔ Baseline saved to ${path}`);
}

function loadSnapshot(name) {
  const path = snapshotPath(name);
  if (!existsSync(path)) {
    throw new Error(
      `Baseline '${name}' not found at ${path}. ` +
        `Run with --save-baseline ${name} first.`,
    );
  }

  let snapshot;
  try {
    snapshot = JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`Failed to parse baseline ${path}: ${message}`);
  }

  if (snapshot.schema !== SNAPSHOT_SCHEMA) {
    throw new Error(
      `Baseline '${name}' has schema ${snapshot.schema}; ` +
        `expected ${SNAPSHOT_SCHEMA}. Regenerate it.`,
    );
  }
  if (snapshot.name !== name || !Array.isArray(snapshot.rows)) {
    throw new Error(`Baseline '${name}' has invalid identity or result rows`);
  }
  if (
    !Number.isFinite(snapshot.timestampUnix) ||
    snapshot.environment === null ||
    typeof snapshot.environment !== "object" ||
    snapshot.fixture === null ||
    typeof snapshot.fixture !== "object" ||
    typeof snapshot.fixture.requestPath !== "string" ||
    !Number.isFinite(snapshot.fixture.protocolBytes)
  ) {
    throw new Error(`Baseline '${name}' is missing required metadata`);
  }
  for (const row of snapshot.rows) {
    const validIdentity =
      typeof row.name === "string" && typeof row.metric === "string";
    const validLatency = [row.p50Ns, row.p95Ns, row.p99Ns, row.meanNs].every(
      (value) => Number.isFinite(value) && value >= 0,
    );
    const validOutputBytes =
      row.outputBytes === null ||
      (Number.isFinite(row.outputBytes) && row.outputBytes >= 0);
    if (!validIdentity || !validLatency || !validOutputBytes) {
      throw new Error(`Baseline '${name}' contains an invalid result row`);
    }
  }
  return snapshot;
}

// ── Human-readable reporting ──────────────────────────────────────────

function formatDuration(nanoseconds) {
  if (nanoseconds < 1_000) return `${nanoseconds.toFixed(0)} ns`;
  if (nanoseconds < 1_000_000) return `${(nanoseconds / 1_000).toFixed(2)} µs`;
  return `${(nanoseconds / 1_000_000).toFixed(2)} ms`;
}

function pctChange(baseline, current) {
  if (baseline === 0) return 0;
  return ((current - baseline) / baseline) * 100;
}

function formatDelta(baseline, current) {
  if (baseline === null || current === null) return "       —";
  return `${pctChange(baseline, current).toFixed(1).padStart(8)}%`;
}

function formatChunkChange(baseline, current) {
  if (baseline === null || current === null) return "       —";
  const value = baseline === current ? String(current) : `${baseline}->${current}`;
  return value.padStart(8);
}

function formatAge(timestampUnix) {
  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - timestampUnix);
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)}h`;
  return `${Math.floor(seconds / 86_400)}d`;
}

function printEnvironmentWarnings(report, baseline) {
  const current = report.environment;
  const previous = baseline.environment ?? {};
  const currentNodeMajor = current.node.split(".")[0];
  const previousNodeMajor = String(previous.node ?? "").split(".")[0];

  if (previous.platform !== current.platform || previous.arch !== current.arch) {
    console.log(
      `WARNING: baseline platform ${previous.platform}-${previous.arch} differs ` +
        `from current ${current.platform}-${current.arch}.`,
    );
  }
  if (previousNodeMajor && previousNodeMajor !== currentNodeMajor) {
    console.log(
      `WARNING: baseline Node ${previous.node} differs from current Node ${current.node}.`,
    );
  }
  if (baseline.fixture?.protocolBytes !== report.fixture.protocolBytes) {
    console.log(
      `WARNING: protocol size changed from ${baseline.fixture?.protocolBytes} to ` +
        `${report.fixture.protocolBytes} bytes; interpret latency deltas with care.`,
    );
  }
  if (baseline.fixture?.requestPath !== report.fixture.requestPath) {
    console.log(
      `WARNING: baseline route ${baseline.fixture?.requestPath} differs from ` +
        `current route ${report.fixture.requestPath}.`,
    );
  }
}

function printDiff(report, baseline) {
  console.log(
    `\nDiff vs baseline '${baseline.name}' (saved ${formatAge(baseline.timestampUnix)} ago):`,
  );
  printEnvironmentWarnings(report, baseline);
  console.log(
    "| case                                     | metric         |" +
      "   P50 Δ% | bytes Δ% |   chunks |",
  );
  console.log(
    "|------------------------------------------|----------------|" +
      "----------|----------|----------|",
  );

  for (const current of snapshotRows(report)) {
    const previous = baseline.rows.find(
      (row) => row.name === current.name && row.metric === current.metric,
    );
    if (!previous) {
      console.log(
        `| ${current.name.padEnd(40)} | ${current.metric.padEnd(14)} |` +
          "    (new) |        — |        — |",
      );
      continue;
    }
    const includeOutputShape = current.metric === "total";
    const bytesDelta = includeOutputShape
      ? formatDelta(previous.outputBytes, current.outputBytes)
      : "       —";
    const chunks = includeOutputShape
      ? formatChunkChange(previous.chunkCount, current.chunkCount)
      : "       —";
    console.log(
      `| ${current.name.padEnd(40)} | ${current.metric.padEnd(14)} | ` +
        `${formatDelta(previous.p50Ns, current.p50Ns)} | ` +
        `${bytesDelta} | ${chunks} |`,
    );
  }

  console.log(
    "\nNegative Δ% = improvement; positive = regression. " +
      "Treat Node P50 changes below ±5% as noise.\n",
  );
}

function printHumanReport(report) {
  console.log("\nWebUI Node addon runtime benchmark");
  console.log(`Addon:    ${report.environment.addonPath}`);
  console.log(
    `Runtime:  Node ${report.environment.node} ` +
      `(${report.environment.platform}-${report.environment.arch})`,
  );
  console.log(
    `Fixture:  contact-book-manager, route ${report.fixture.requestPath}, protocol ` +
      `${report.fixture.protocolBytes.toLocaleString("en-US")} bytes`,
  );
  console.log(`Mode:     ${report.config.quick ? "quick smoke" : "full"}\n`);

  console.log("Workloads:");
  console.log("| contacts | state bytes | HTML bytes | stream chunks |");
  console.log("|---------:|------------:|-----------:|--------------:|");
  for (const workload of report.workloads) {
    console.log(
      `| ${String(workload.contacts).padStart(8)} ` +
        `| ${workload.stateBytes.toLocaleString("en-US").padStart(11)} ` +
        `| ${workload.htmlBytes.toLocaleString("en-US").padStart(10)} ` +
        `| ${String(workload.streamChunks).padStart(13)} |`,
    );
  }

  console.log("\nLatency:");
  console.log(
    "| case                               | metric         | samples |" +
      "        P50 |        P95 |        P99 |      ops/s |",
  );
  console.log(
    "|------------------------------------|----------------|--------:|" +
      "-----------:|-----------:|-----------:|-----------:|",
  );
  for (const result of report.results) {
    const meanSeconds = result.latency.meanNs / 1_000_000_000;
    const throughput =
      result.metric === "total" ? (1 / meanSeconds).toFixed(1) : "—";
    console.log(
      `| ${result.name.padEnd(34)} | ${result.metric.padEnd(14)} ` +
        `| ${String(result.latency.samples).padStart(7)} ` +
        `| ${formatDuration(result.latency.p50Ns).padStart(10)} ` +
        `| ${formatDuration(result.latency.p95Ns).padStart(10)} ` +
        `| ${formatDuration(result.latency.p99Ns).padStart(10)} ` +
        `| ${throughput.padStart(10)} |`,
    );
  }
  console.log("\nops/s is derived from mean total latency; first-callback is not completion.\n");
}

// ── Benchmark orchestration ───────────────────────────────────────────

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    usage();
    return;
  }
  validateOptions(options);
  const comparisonBaseline = options.compareBaseline
    ? loadSnapshot(options.compareBaseline)
    : null;

  const config = options.quick
    ? {
        quick: true,
        warmupIterations: 2,
        minSamples: 3,
        maxSamples: 3,
        targetDurationNs: 0,
      }
    : {
        quick: false,
        warmupIterations: 10,
        minSamples: 20,
        maxSamples: 20_000,
        targetDurationNs: 750_000_000,
      };

  // Resolve the native artifact before loading the public package.
  const { resolve: resolveArtifact } = await import("@microsoft/webui/platform.js");
  const addonPath = resolveArtifact("addon");
  if (!addonPath) {
    throw new Error(
      "Node addon not found. Run: cargo build --release -p microsoft-webui-node",
    );
  }

  const normalizedAddonPath = addonPath.replaceAll("\\", "/");
  if (normalizedAddonPath.includes("/target/debug/") && !options.allowDebug) {
    throw new Error(
      `Refusing to benchmark debug addon ${addonPath}. ` +
        "Build release or pass --allow-debug for a smoke run.",
    );
  }

  // Lock the public package to the artifact reported above before it loads.
  process.env.WEBUI_ADDON_PATH = addonPath;
  const { build, Protocol } = await import("@microsoft/webui");

  // Build and validate the live fixture outside every timed region.
  const appDir = fileURLToPath(
    new URL("../../app/contact-book-manager/src/", import.meta.url),
  );
  const statePath = fileURLToPath(
    new URL("../../app/contact-book-manager/data/state.json", import.meta.url),
  );
  const seedState = JSON.parse(readFileSync(statePath, "utf8"));
  if (!options.json) {
    console.error("Preparing contact-book protocol and benchmark states...");
  }
  const buildResult = build({ appDir, css: "style" });
  const protocolBytes = buildResult.protocol;
  assert.ok(protocolBytes.length > 0, "build returned an empty protocol");
  const protocol = new Protocol(protocolBytes);

  const fixtures = CONTACT_COUNTS.map((contactCount) => {
    const state = buildState(seedState, contactCount);
    const stateJson = JSON.stringify(state);
    const expected = protocol.render(stateJson, RENDER_OPTIONS);
    assert.equal(
      protocol.render(state, RENDER_OPTIONS),
      expected,
      `object and JSON-string render differ at ${contactCount} contacts`,
    );
    const chunks = [];
    protocol.renderStream(
      stateJson,
      (chunk) => {
        chunks.push(chunk);
      },
      RENDER_OPTIONS,
    );
    assert.equal(
      chunks.join(""),
      expected,
      `renderStream output differs at ${contactCount} contacts`,
    );
    return {
      contactCount,
      state,
      stateJson,
      expected,
      inputBytes: Buffer.byteLength(stateJson),
      outputBytes: Buffer.byteLength(expected),
      chunkCount: chunks.length,
    };
  });

  // Measure construction once, then the reusable Protocol render paths.
  const results = [];
  results.push(
    makeResult(
      "protocol/new",
      "total",
      collectSyncSamples(() => new Protocol(protocolBytes), config),
      { inputBytes: protocolBytes.length },
    ),
  );

  for (const fixture of fixtures) {
    const details = {
      contacts: fixture.contactCount,
      inputBytes: fixture.inputBytes,
      outputBytes: fixture.outputBytes,
    };

    results.push(
      makeResult(
        `render/json-string/${fixture.contactCount}`,
        "total",
        collectSyncSamples(
          () => protocol.render(fixture.stateJson, RENDER_OPTIONS),
          config,
        ),
        details,
      ),
    );
    results.push(
      makeResult(
        `render/object/${fixture.contactCount}`,
        "total",
        collectSyncSamples(
          () => protocol.render(fixture.state, RENDER_OPTIONS),
          config,
        ),
        details,
      ),
    );

    const stream = collectStreamSamples(
      protocol,
      fixture.stateJson,
      fixture.expected.length,
      config,
    );
    assert.equal(
      stream.chunkCount,
      fixture.chunkCount,
      `streaming chunk count changed at ${fixture.contactCount} contacts`,
    );
    results.push(
      makeResult(
        `render-stream/json-string/${fixture.contactCount}`,
        "first-callback",
        stream.firstCallback,
        { ...details, chunkCount: stream.chunkCount },
      ),
    );
    results.push(
      makeResult(
        `render-stream/json-string/${fixture.contactCount}`,
        "total",
        stream.total,
        { ...details, chunkCount: stream.chunkCount },
      ),
    );
  }

  // Keep the raw report data-only so --json and snapshots stay machine-readable.
  const report = {
    schemaVersion: 1,
    benchmark: "node-addon-runtime",
    environment: {
      addonPath,
      node: process.versions.node,
      napi: process.versions.napi,
      platform: process.platform,
      arch: process.arch,
    },
    config,
    fixture: {
      name: "contact-book-manager",
      requestPath: RENDER_OPTIONS.requestPath,
      protocolBytes: protocolBytes.length,
      contactCounts: CONTACT_COUNTS,
    },
    workloads: fixtures.map((fixture) => ({
      contacts: fixture.contactCount,
      stateBytes: fixture.inputBytes,
      htmlBytes: fixture.outputBytes,
      streamChunks: fixture.chunkCount,
    })),
    results,
  };

  // Human output, raw JSON, and baseline modes all consume the same report.
  if (options.json) {
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    return;
  }

  printHumanReport(report);
  if (comparisonBaseline) {
    printDiff(report, comparisonBaseline);
  }
  if (options.saveBaseline) {
    saveSnapshot(options.saveBaseline, report);
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
