// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * FFI CPU benchmark for the WebUI Node.js native addon.
 *
 * Load tests reported the Node SSR host burning ~60% CPU vs ~25% for the
 * control at 150 RPS, with traces blaming JSON processing of the state object.
 * The sibling Rust bench (`crates/webui/examples/state_cpu_bench.rs`) isolates
 * parse vs render CPU *inside* Rust. This harness measures the **full FFI
 * per-request CPU** as Node actually pays it, including the costs a pure-Rust
 * bench cannot see:
 *
 *   1. napi copies `stateJson` from V8 into a Rust `String`.
 *   2. `serde_json::from_str` builds the owned state tree.
 *   3. `WebUIHandler::render` walks the protocol.
 *   4. every emitted chunk crosses the FFI boundary back into JS
 *      (`content.to_owned()` per chunk + the `onChunk` call).
 *
 * It uses `process.cpuUsage()` (user + system microseconds) around a tight
 * `render()` loop over increasingly large JSON state, and reports user µs/op,
 * system µs/op, wall µs/op, CPU% and state throughput (MiB/s).
 *
 * The addon is loaded directly from the cargo build output via `process.dlopen`
 * — no napi packaging step required. Build it first:
 *
 *   cargo build -p microsoft-webui-node --release
 *
 * Then run:
 *
 *   node crates/webui-node/bench/ffi_cpu_bench.mjs
 *   node crates/webui-node/bench/ffi_cpu_bench.mjs --save main
 *   node crates/webui-node/bench/ffi_cpu_bench.mjs --compare main
 *
 * or via xtask (which builds the addon for you):
 *
 *   cargo xtask bench ffi-cpu
 *   cargo xtask bench ffi-cpu --save-baseline main
 *   cargo xtask bench ffi-cpu --baseline main
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SNAPSHOT_SCHEMA = 1;
const SCALES = [1_000, 5_000, 10_000];
const ITERS_PER_SCALE = 400;

const HERE = path.dirname(fileURLToPath(import.meta.url));
// crates/webui-node/bench -> repo root
const REPO_ROOT = path.resolve(HERE, "..", "..", "..");

// ── Addon loading ─────────────────────────────────────────────────────

function addonCandidates() {
  const base = path.join(REPO_ROOT, "target", "release");
  switch (process.platform) {
    case "win32":
      return [path.join(base, "webui_node.dll")];
    case "darwin":
      return [
        path.join(base, "libwebui_node.dylib"),
        path.join(base, "webui_node.dylib"),
      ];
    default:
      return [
        path.join(base, "libwebui_node.so"),
        path.join(base, "webui_node.so"),
      ];
  }
}

function loadAddon() {
  for (const candidate of addonCandidates()) {
    if (fs.existsSync(candidate)) {
      const m = { exports: {} };
      process.dlopen(m, candidate);
      return { addon: m.exports, path: candidate };
    }
  }
  const looked = addonCandidates()
    .map((p) => `  - ${p}`)
    .join("\n");
  console.error(
    `FFI addon not found. Build it first:\n\n` +
      `  cargo build -p microsoft-webui-node --release\n\n` +
      `Looked for:\n${looked}`,
  );
  process.exit(2);
}

// ── State fixtures (mirror state_cpu_bench.rs) ────────────────────────

const FIRST_NAMES = [
  "Ava", "Liam", "Noah", "Emma", "Mia", "Ethan", "Sofia", "Lucas", "Aria", "Mateo",
];
const LAST_NAMES = [
  "Nguyen", "Johnson", "Tanaka", "Sharma", "O'Brien", "Okafor", "Ramirez", "Lindström", "Kim", "Al-Hassan",
];
const GROUPS = ["Family", "Work", "Friends", "Other"];

function generateContact(idx) {
  const first = FIRST_NAMES[idx % FIRST_NAMES.length];
  const last = LAST_NAMES[idx % LAST_NAMES.length];
  const phone3 = String((idx * 111) % 1000).padStart(3, "0");
  const phone4 = String((idx * 1234) % 10000).padStart(4, "0");
  return {
    id: String(idx + 1),
    firstName: first,
    lastName: last,
    email: `${first.toLowerCase()}.${last.toLowerCase()}@example.com`,
    phone: `+1 (555) ${phone3}-${phone4}`,
    company: "Contoso Ltd",
    group: GROUPS[idx % GROUPS.length],
    favorite: idx % 3 === 0,
    initials: `${first[0]}${last[0]}`,
    avatarColor: "#4A90D9",
    notes: "",
    address: `${(idx + 1) * 100} St, Seattle, WA`,
  };
}

function buildState(count) {
  const contacts = [];
  for (let i = 0; i < count; i += 1) contacts.push(generateContact(i));
  const recent = contacts.slice(Math.max(0, count - 5));
  return {
    page: "dashboard",
    searchQuery: "",
    activeGroup: "all",
    groups: GROUPS,
    totalContacts: count,
    totalFavorites: 0,
    totalGroups: GROUPS.length,
    contacts,
    filteredContacts: contacts,
    recentContacts: recent,
    favoriteContacts: [],
    selectedContact: null,
  };
}

// ── Measurement ───────────────────────────────────────────────────────

/**
 * Run `render()` `iters` times and capture CPU + wall deltas.
 * `onChunk` only sums byte length so the per-chunk FFI callback cost is
 * measured but JS-side work stays negligible.
 */
function measure(addon, protocol, stateJson, entry, iters) {
  let outBytes = 0;
  let chunks = 0;
  const onChunk = (html) => {
    outBytes += html.length;
    chunks += 1;
  };

  // Warm up: JIT + first-call lazy init.
  for (let i = 0; i < 3; i += 1) addon.render(protocol, stateJson, entry, "/", onChunk);

  outBytes = 0;
  chunks = 0;
  const cpu0 = process.cpuUsage();
  const wall0 = process.hrtime.bigint();
  for (let i = 0; i < iters; i += 1) {
    addon.render(protocol, stateJson, entry, "/", onChunk);
  }
  const wallNs = Number(process.hrtime.bigint() - wall0);
  const cpu = process.cpuUsage(cpu0); // { user, system } in microseconds

  const n = iters;
  const userUs = cpu.user / n;
  const sysUs = cpu.system / n;
  const wallUs = wallNs / 1000 / n;
  const cpuPct = wallUs > 0 ? ((userUs + sysUs) / wallUs) * 100 : 0;
  const wallS = wallNs / 1e9 / n;
  const stateMiBs = wallS > 0 ? stateJson.length / (1024 * 1024) / wallS : 0;

  return {
    iters,
    userUs,
    sysUs,
    wallUs,
    cpuPct,
    stateMiBs,
    outBytesPerOp: outBytes / n,
    chunksPerOp: chunks / n,
  };
}

// ── Reporting ─────────────────────────────────────────────────────────

function printHeader() {
  console.log();
  console.log(
    `| ${"scale".padEnd(9)} | ${"iters".padStart(6)} | ${"user µs/op".padStart(11)} | ` +
      `${"sys µs/op".padStart(10)} | ${"wall µs/op".padStart(11)} | ${"CPU%".padStart(6)} | ` +
      `${"state MiB/s".padStart(11)} | ${"out KiB/op".padStart(10)} | ${"chunks/op".padStart(9)} |`,
  );
  console.log(
    `|${"-".repeat(11)}|${"-".repeat(8)}|${"-".repeat(13)}|${"-".repeat(12)}|` +
      `${"-".repeat(13)}|${"-".repeat(8)}|${"-".repeat(13)}|${"-".repeat(12)}|${"-".repeat(11)}|`,
  );
}

function printRow(scale, r) {
  console.log(
    `| ${String(scale).padEnd(9)} | ${String(r.iters).padStart(6)} | ` +
      `${r.userUs.toFixed(2).padStart(11)} | ${r.sysUs.toFixed(2).padStart(10)} | ` +
      `${r.wallUs.toFixed(2).padStart(11)} | ${`${r.cpuPct.toFixed(0)}%`.padStart(6)} | ` +
      `${r.stateMiBs.toFixed(1).padStart(11)} | ${(r.outBytesPerOp / 1024).toFixed(1).padStart(10)} | ` +
      `${r.chunksPerOp.toFixed(1).padStart(9)} |`,
  );
}

// ── Baselines ─────────────────────────────────────────────────────────

function snapshotPath(name) {
  return path.join(REPO_ROOT, "target", "bench-baselines", `ffi-cpu-${name}.json`);
}

function saveSnapshot(name, rows) {
  const file = snapshotPath(name);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const snap = {
    schema: SNAPSHOT_SCHEMA,
    name,
    timestampUnix: Math.floor(Date.now() / 1000),
    rows,
  };
  fs.writeFileSync(file, JSON.stringify(snap, null, 2));
  console.log();
  console.log(`✔ Baseline saved to ${file}`);
}

function loadSnapshot(name) {
  const file = snapshotPath(name);
  if (!fs.existsSync(file)) {
    console.error(`compare: baseline '${name}' not found at ${file} — run with --save ${name} first`);
    return null;
  }
  const snap = JSON.parse(fs.readFileSync(file, "utf8"));
  if (snap.schema !== SNAPSHOT_SCHEMA) {
    console.error(`compare: baseline '${name}' schema ${snap.schema} (expected ${SNAPSHOT_SCHEMA}); regenerate with --save`);
    return null;
  }
  return snap;
}

function pctChange(base, current) {
  if (base === 0) return 0;
  return ((current - base) / base) * 100;
}

function printDiff(rows, baseline) {
  console.log();
  console.log(`Diff vs baseline '${baseline.name}'`);
  console.log(
    `| ${"scale".padEnd(9)} | ${"user_cpu Δ%".padStart(14)} | ${"wall Δ%".padStart(14)} | ${"CPU% Δ (pts)".padStart(14)} |`,
  );
  console.log(`|${"-".repeat(11)}|${"-".repeat(16)}|${"-".repeat(16)}|${"-".repeat(16)}|`);
  for (const cur of rows) {
    const base = baseline.rows.find((b) => b.scale === cur.scale);
    if (!base) {
      console.log(`| ${String(cur.scale).padEnd(9)} | ${"(new row)".padStart(14)} | ${"—".padStart(14)} | ${"—".padStart(14)} |`);
      continue;
    }
    const ud = pctChange(base.userUs, cur.userUs);
    const wd = pctChange(base.wallUs, cur.wallUs);
    const cd = cur.cpuPct - base.cpuPct;
    console.log(
      `| ${String(cur.scale).padEnd(9)} | ${`${ud.toFixed(1)}%`.padStart(14)} | ${`${wd.toFixed(1)}%`.padStart(14)} | ${`${cd.toFixed(1)}`.padStart(14)} |`,
    );
  }
  console.log();
  console.log("Negative Δ% = improvement; positive = regression. Threshold for action: ±5%.");
  console.log();
}

// ── CLI ───────────────────────────────────────────────────────────────

function parseArgs() {
  const args = process.argv.slice(2);
  for (let i = 0; i < args.length; i += 1) {
    if (args[i] === "--save") return { mode: "save", name: args[i + 1] };
    if (args[i] === "--compare") return { mode: "compare", name: args[i + 1] };
    if (args[i] === "--help" || args[i] === "-h") {
      console.log(
        "Usage: node ffi_cpu_bench.mjs [--save NAME] [--compare NAME]\n\n" +
          "  With no args: prints the FFI CPU table.\n" +
          "  --save NAME: write results to target/bench-baselines/ffi-cpu-NAME.json\n" +
          "  --compare NAME: print results AND a Δ%-table vs the saved baseline",
      );
      process.exit(0);
    }
  }
  return { mode: "print", name: null };
}

// ── Main ──────────────────────────────────────────────────────────────

function main() {
  const { mode, name } = parseArgs();
  if ((mode === "save" || mode === "compare") && !name) {
    console.error(`--${mode} requires a baseline name`);
    process.exit(2);
  }

  const { addon, path: addonPath } = loadAddon();
  const appDir = path.join(REPO_ROOT, "examples", "app", "contact-book-manager", "src");
  const entry = "index.html";
  const built = addon.build({ appDir, entry, css: "style" });
  const protocol = built.protocol;

  console.log("WebUI FFI CPU benchmark (native addon render, process.cpuUsage)");
  console.log("==============================================================");
  console.log(`Addon:   ${addonPath}`);
  console.log(`Node:    ${process.version} | platform: ${process.platform}-${process.arch}`);
  console.log(`Iters per scale: ${ITERS_PER_SCALE} | protocol: ${protocol.length} bytes`);
  console.log("CPU% = (user+sys)/wall. This includes napi string marshalling + per-chunk FFI callbacks.");
  printHeader();

  const rows = [];
  for (const scale of SCALES) {
    const stateJson = JSON.stringify(buildState(scale));
    const r = measure(addon, protocol, stateJson, entry, ITERS_PER_SCALE);
    printRow(scale, r);
    rows.push({ scale, ...r });
  }

  console.log();
  console.log("Notes:");
  console.log("  * `user`/`sys µs/op` come from process.cpuUsage() deltas / iters.");
  console.log("  * `state MiB/s` normalizes throughput to the input state-JSON size.");
  console.log("  * This is the full per-request FFI cost; the Rust example splits it");
  console.log("    into parse vs render (cargo xtask bench state-cpu).");

  if (mode === "save") saveSnapshot(name, rows);
  else if (mode === "compare") {
    const baseline = loadSnapshot(name);
    if (baseline) printDiff(rows, baseline);
  }
}

main();
