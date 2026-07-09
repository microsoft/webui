// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * FFI CPU benchmark for the WebUI Node.js native addon — A/B vs a JS control.
 *
 * Load tests reported the Node SSR host burning ~60% CPU vs ~25% for the
 * control at 150 RPS, with traces blaming JSON processing of the state object.
 * To *prove* whether the FFI path is fast or a regression vs "plain JS", this
 * harness measures three arms per request, all from the SAME live state object:
 *
 *   A. webui (stringify+render) — the true per-request cost: the caller must
 *      `JSON.stringify(state)` because `render()` takes a JSON string, then Rust
 *      re-parses it with `serde_json::from_str` into an owned `Value` tree and
 *      walks the protocol. This is what a Node host pays today.
 *   A'. webui (render only) — render on a *prebuilt* JSON string. This is what
 *      the old harness measured; the gap A − A' is the hidden `JSON.stringify`
 *      tax the old numbers omitted.
 *   C. control (pure JS, live object) — a hand-written SSR of the SAME dashboard
 *      route straight from the live object: no stringify, no parse. This is the
 *      "control uses JS to do everything" baseline the load test compared to.
 *
 * The `/` route only emits a fixed-size dashboard (stats + sidebar + the 5
 * `recentContacts` cards), so the control's render work does NOT grow with the
 * contact count — but WebUI still parses the entire state on every request. The
 * A − C gap is therefore the JSON tax, and it is exactly the CPU the load test
 * saw. This harness doubles as a regression gate for CPU utilisation, latency
 * and throughput of the render path (compare with --save/--compare).
 *
 * Reports user µs/op, system µs/op, wall µs/op (latency), ops/s (single-thread
 * render throughput), CPU% (on-core saturation), core%@150 (projected % of one
 * core at 150 RPS — the load-test framing) and out KiB/op for each arm, plus a
 * "webui = N× control" CPU ratio, the isolated stringify tax, and a host memory
 * (RSS/V8 heap) working-set probe on the render loop. Exact per-op render
 * allocation (allocs/op, bytes/op) is deterministic and lives in the Rust
 * `state-cpu` bench, since the serde tree is freed inside each native call.
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

const SNAPSHOT_SCHEMA = 3;
const SCALES = [1_000, 5_000, 10_000];
// Per-arm iteration counts are calibrated at runtime so each arm accumulates
// enough CPU to clear the OS CPU-clock tick (~15 ms on Windows). Without this,
// the sub-10-µs control arm reads as "0 µs / 0% CPU". Each arm runs until it has
// spent ~TARGET_MS of wall time, bounded by [MIN_ITERS, MAX_ITERS].
const TARGET_MS = 1_500;
const MIN_ITERS = 20;
const MAX_ITERS = 2_000_000;
// Load-test framing: the report was "~60% of a core vs ~25% at 150 RPS". At a
// fixed request rate the share of ONE core an arm consumes is just its
// per-request CPU time × RPS, so we project each arm's (user+sys) CPU into
// "% of one core @ TARGET_RPS" — the number directly comparable to that report.
const TARGET_RPS = 150;

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

// ── Control renderer (pure JS SSR of the `/` dashboard, no JSON round-trip) ──

/** HTML-escape interpolated text, mirroring the work WebUI does per value. */
function esc(value) {
  const s = String(value);
  let out = "";
  let last = 0;
  for (let i = 0; i < s.length; i += 1) {
    const ch = s.charCodeAt(i);
    let rep;
    if (ch === 38) rep = "&amp;";
    else if (ch === 60) rep = "&lt;";
    else if (ch === 62) rep = "&gt;";
    else if (ch === 34) rep = "&quot;";
    else continue;
    out += s.slice(last, i) + rep;
    last = i + 1;
  }
  return last === 0 ? s : out + s.slice(last);
}

function renderCard(c) {
  let o =
    `<cb-contact-card id="${esc(c.id)}" first-name="${esc(c.firstName)}" ` +
    `last-name="${esc(c.lastName)}" email="${esc(c.email)}" phone="${esc(c.phone)}" ` +
    `company="${esc(c.company)}" group="${esc(c.group)}" favorite="${c.favorite}" ` +
    `initials="${esc(c.initials)}" avatar-color="${esc(c.avatarColor)}" ` +
    `notes="${esc(c.notes)}" address="${esc(c.address)}">`;
  o += `<template shadowrootmode="open"><a class="card" href="./contacts/${esc(c.id)}">`;
  o += `<div class="avatar" style="background-color: ${esc(c.avatarColor)}">`;
  o += `<span class="avatar-initials">${esc(c.initials)}</span></div>`;
  o += `<div class="card-content"><div class="card-top">`;
  o += `<span class="name">${esc(c.firstName)} ${esc(c.lastName)}</span>`;
  if (c.favorite === true) o += `<span class="fav-star"></span>`;
  o += `</div><span class="email">${esc(c.email)}</span>`;
  o += `<span class="phone">${esc(c.phone)}</span></div>`;
  o += `<span class="badge">${esc(c.group)}</span></a></template></cb-contact-card>`;
  return o;
}

/** Render the same dashboard route WebUI serves for "/", from the live object. */
function renderControl(s) {
  let out = `<cb-header search-query="${esc(s.searchQuery)}"><template shadowrootmode="open">`;
  out += `<div class="header-left"><h1 class="title">Contacts</h1></div>`;
  out += `<div class="header-center"><div class="search-container"><span class="search-icon"></span>`;
  out += `<input class="search-input" type="text" placeholder="Search contacts..." value="${esc(s.searchQuery)}" /></div></div>`;
  out += `<div class="header-right"><a class="add-btn" href="./contacts/add">`;
  out += `<span class="add-icon">+</span><span class="add-label">Add Contact</span></a></div>`;
  out += `</template></cb-header><div class="layout">`;
  out +=
    `<cb-sidebar page="${esc(s.page)}" active-group="${esc(s.activeGroup)}" ` +
    `total-contacts="${s.totalContacts}" total-favorites="${s.totalFavorites}">`;
  out += `<template shadowrootmode="open"><div class="nav-section">`;
  out += `<a class="nav-item" data-nav="Dashboard" href="./"><span class="nav-icon nav-icon-dashboard"></span><span class="nav-label">Dashboard</span></a>`;
  out += `<a class="nav-item" data-nav="All Contacts" href="./contacts"><span class="nav-icon nav-icon-contacts"></span><span class="nav-label">All Contacts</span><span class="nav-count">${s.totalContacts}</span></a>`;
  out += `<a class="nav-item" data-nav="Favorites" href="./favorites"><span class="nav-icon nav-icon-favorites"></span><span class="nav-label">Favorites</span><span class="nav-count">${s.totalFavorites}</span></a>`;
  out += `</div><div class="nav-divider"></div><div class="nav-section"><h3 class="nav-heading">Groups</h3>`;
  for (const group of s.groups) {
    out += `<a class="nav-item nav-item-group" data-nav="${esc(group)}" href="./groups/${esc(group)}">`;
    out += `<span class="nav-icon nav-icon-folder"></span><span class="nav-label">${esc(group)}</span></a>`;
  }
  out += `</div></template></cb-sidebar><main class="content">`;
  out += `<h2 class="page-title">Dashboard</h2><div class="stats-row">`;
  out += `<div class="stat-card"><span class="stat-icon stat-icon-contacts"></span><div class="stat-content"><span class="stat-value">${s.totalContacts}</span><span class="stat-label">Total Contacts</span></div></div>`;
  out += `<div class="stat-card"><span class="stat-icon stat-icon-favorites"></span><div class="stat-content"><span class="stat-value">${s.totalFavorites}</span><span class="stat-label">Favorites</span></div></div>`;
  out += `<div class="stat-card"><span class="stat-icon stat-icon-groups"></span><div class="stat-content"><span class="stat-value">${s.totalGroups}</span><span class="stat-label">Groups</span></div></div>`;
  out += `</div><h3 class="section-title">Recent Contacts</h3><div class="contact-list-container">`;
  for (const c of s.recentContacts) out += renderCard(c);
  out += `</div></main></div>`;
  return out;
}

// ── Measurement ───────────────────────────────────────────────────────

/**
 * Estimate how many iterations of `thunk` fit in ~TARGET_MS of wall time, so a
 * cheap arm runs long enough for CPU accounting to be meaningful. Returns a
 * count clamped to [MIN_ITERS, MAX_ITERS].
 */
function calibrateIters(thunk) {
  const probe = 8;
  for (let i = 0; i < 3; i += 1) thunk(); // warm up JIT + first-call lazy init
  const t0 = process.hrtime.bigint();
  for (let i = 0; i < probe; i += 1) thunk();
  const perOpMs = Number(process.hrtime.bigint() - t0) / 1e6 / probe;
  if (perOpMs <= 0) return MAX_ITERS;
  const n = Math.ceil(TARGET_MS / perOpMs);
  return Math.min(MAX_ITERS, Math.max(MIN_ITERS, n));
}

/**
 * Run `thunk` `iters` times and capture CPU + wall deltas. The thunk returns the
 * number of output bytes it produced so we can report per-op output size.
 */
function measureThunk(thunk, iters) {
  for (let i = 0; i < 3; i += 1) thunk(); // warm up JIT + first-call lazy init

  let outBytes = 0;
  const cpu0 = process.cpuUsage();
  const wall0 = process.hrtime.bigint();
  for (let i = 0; i < iters; i += 1) outBytes += thunk();
  const wallNs = Number(process.hrtime.bigint() - wall0);
  const cpu = process.cpuUsage(cpu0); // { user, system } microseconds

  const userUs = cpu.user / iters;
  const sysUs = cpu.system / iters;
  const wallUs = wallNs / 1000 / iters;
  return {
    iters,
    userUs,
    sysUs,
    wallUs,
    cpuPct: wallUs > 0 ? ((userUs + sysUs) / wallUs) * 100 : 0,
    outBytesPerOp: outBytes / iters,
  };
}

/** Calibrate then measure a single arm. */
function runArm(thunk) {
  return measureThunk(thunk, calibrateIters(thunk));
}

/**
 * Probe host memory for a render loop. The per-request serde tree is allocated
 * AND freed inside one synchronous native `render()` call, so it never surfaces
 * in `process.memoryUsage()` — what we can observe is the process **working
 * set** (RSS) and V8 heap high-water under sustained load. Exact per-op render
 * allocation (allocs/op, bytes/op) is deterministic and lives in the Rust
 * `state-cpu` bench. GC is forced around the probe when `--expose-gc` is set so
 * the baseline/settled figures are stable rather than GC-timing artefacts.
 */
function measureMemory(thunk, iters) {
  if (global.gc) global.gc();
  const base = process.memoryUsage();
  let peakRss = base.rss;
  let peakHeap = base.heapUsed;
  for (let i = 0; i < iters; i += 1) {
    thunk();
    const m = process.memoryUsage();
    if (m.rss > peakRss) peakRss = m.rss;
    if (m.heapUsed > peakHeap) peakHeap = m.heapUsed;
  }
  if (global.gc) global.gc();
  const settled = process.memoryUsage();
  return {
    gc: Boolean(global.gc),
    baseRss: base.rss,
    peakRss,
    settledRss: settled.rss,
    peakHeap,
  };
}

/** Build the measured arms for one scale. */
function measureScale(addon, protocol, entry, state) {
  const prebuilt = JSON.stringify(state);
  let chunkBytes = 0;
  const onChunk = (html) => {
    chunkBytes += html.length;
  };

  const webuiTotal = runArm(() => {
    chunkBytes = 0;
    const json = JSON.stringify(state);
    addon.render(protocol, json, entry, "/", onChunk);
    return chunkBytes;
  });

  const stringifyOnly = runArm(() => JSON.stringify(state).length);

  const webuiRender = runArm(() => {
    chunkBytes = 0;
    addon.render(protocol, prebuilt, entry, "/", onChunk);
    return chunkBytes;
  });

  const control = runArm(() => renderControl(state).length);

  // Host working-set probe on the full string-path render (the real per-request
  // path). Kept out of the timed arms above so it never perturbs their CPU.
  const memory = measureMemory(() => {
    chunkBytes = 0;
    const json = JSON.stringify(state);
    addon.render(protocol, json, entry, "/", onChunk);
  }, 300);

  return {
    stateBytes: prebuilt.length,
    arms: { webuiTotal, stringifyOnly, webuiRender, control },
    memory,
  };
}

// ── Reporting ─────────────────────────────────────────────────────────

const ARM_LABELS = {
  webuiTotal: "webui (stringify+render)",
  stringifyOnly: "  stringify only",
  webuiRender: "  webui (render only)",
  control: "control (pure JS)",
};

/** Compact throughput formatting: 402000 → "402k", 1234 → "1.2k", 20 → "20". */
function formatOps(ops) {
  if (ops >= 10_000) return `${(ops / 1000).toFixed(0)}k`;
  if (ops >= 1_000) return `${(ops / 1000).toFixed(1)}k`;
  return ops.toFixed(0);
}

function printScale(scale, res) {
  console.log();
  console.log(
    `Scale ${scale} contacts | state JSON ${(res.stateBytes / (1024 * 1024)).toFixed(2)} MiB`,
  );
  console.log(
    `| ${"arm".padEnd(24)} | ${"iters".padStart(8)} | ${"user µs/op".padStart(11)} | ${"sys µs/op".padStart(10)} | ` +
      `${"wall µs/op".padStart(11)} | ${"ops/s".padStart(9)} | ${"CPU%".padStart(6)} | ${"core%@150".padStart(10)} | ${"out KiB/op".padStart(10)} |`,
  );
  console.log(
    `|${"-".repeat(26)}|${"-".repeat(10)}|${"-".repeat(13)}|${"-".repeat(12)}|${"-".repeat(13)}|${"-".repeat(11)}|${"-".repeat(8)}|${"-".repeat(12)}|${"-".repeat(12)}|`,
  );
  for (const key of ["webuiTotal", "stringifyOnly", "webuiRender", "control"]) {
    const r = res.arms[key];
    const corePct = ((r.userUs + r.sysUs) * TARGET_RPS) / 10_000;
    const coreStr = corePct >= 10 ? corePct.toFixed(0) : corePct.toFixed(2);
    const opsPerSec = r.wallUs > 0 ? 1_000_000 / r.wallUs : 0;
    console.log(
      `| ${ARM_LABELS[key].padEnd(24)} | ${String(r.iters).padStart(8)} | ${r.userUs.toFixed(2).padStart(11)} | ` +
        `${r.sysUs.toFixed(2).padStart(10)} | ${r.wallUs.toFixed(2).padStart(11)} | ` +
        `${formatOps(opsPerSec).padStart(9)} | ${`${r.cpuPct.toFixed(0)}%`.padStart(6)} | ${`${coreStr}%`.padStart(10)} | ` +
        `${(r.outBytesPerOp / 1024).toFixed(2).padStart(10)} |`,
    );
  }

  const total = res.arms.webuiTotal.userUs;
  const stringify = res.arms.stringifyOnly.userUs;
  const render = res.arms.webuiRender.userUs;
  const ctrl = res.arms.control.userUs;
  const ratio = ctrl > 0 ? (total / ctrl).toFixed(0) : "∞";
  const stringifyPct = total > 0 ? (stringify / total) * 100 : 0;
  console.log(
    `  → webui burns ${ratio}× the control's user CPU. ` +
      `stringify ≈ ${stringify.toFixed(0)} µs/op (${stringifyPct.toFixed(0)}% of webui); ` +
      `serde parse+render ≈ ${render.toFixed(0)} µs/op vs control ${ctrl.toFixed(1)} µs/op.`,
  );
  const webuiCore = ((res.arms.webuiTotal.userUs + res.arms.webuiTotal.sysUs) * TARGET_RPS) / 10_000;
  const ctrlCore = ((res.arms.control.userUs + res.arms.control.sysUs) * TARGET_RPS) / 10_000;
  console.log(
    `    load-test projection @${TARGET_RPS} RPS: webui ≈ ${webuiCore.toFixed(0)}% of one core ` +
      `vs control ≈ ${ctrlCore.toFixed(2)}% (${ctrlCore > 0 ? (webuiCore / ctrlCore).toFixed(0) : "∞"}× the core utilisation).`,
  );
  const mem = res.memory;
  const miB = (bytes) => (bytes / (1024 * 1024)).toFixed(1);
  console.log(
    `    host memory (webui render loop): RSS ${miB(mem.baseRss)}→${miB(mem.peakRss)} MiB peak, ` +
      `settled ${miB(mem.settledRss)} MiB; V8 heap peak ${miB(mem.peakHeap)} MiB` +
      `${mem.gc ? "" : " (run node --expose-gc for stable figures)"}.`,
  );
  console.log(
    "      note: the per-op serde tree is transient (freed each call) — exact allocs/op & bytes/op are in `cargo xtask bench state-cpu`.",
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
  console.log(`Diff vs baseline '${baseline.name}' (webui stringify+render arm, user CPU)`);
  console.log(
    `| ${"scale".padEnd(9)} | ${"user Δ%".padStart(12)} | ${"wall Δ%".padStart(12)} | ${"webui/control Δ".padStart(16)} |`,
  );
  console.log(`|${"-".repeat(11)}|${"-".repeat(14)}|${"-".repeat(14)}|${"-".repeat(18)}|`);
  for (const cur of rows) {
    const base = baseline.rows.find((b) => b.scale === cur.scale);
    if (!base) {
      console.log(`| ${String(cur.scale).padEnd(9)} | ${"(new row)".padStart(12)} | ${"—".padStart(12)} | ${"—".padStart(16)} |`);
      continue;
    }
    const ct = cur.arms.webuiTotal;
    const bt = base.arms.webuiTotal;
    const ud = pctChange(bt.userUs, ct.userUs);
    const wd = pctChange(bt.wallUs, ct.wallUs);
    const curRatio = cur.arms.control.userUs > 0 ? ct.userUs / cur.arms.control.userUs : 0;
    const baseRatio = base.arms.control.userUs > 0 ? bt.userUs / base.arms.control.userUs : 0;
    const rd = curRatio - baseRatio;
    console.log(
      `| ${String(cur.scale).padEnd(9)} | ${`${ud.toFixed(1)}%`.padStart(12)} | ${`${wd.toFixed(1)}%`.padStart(12)} | ${`${rd >= 0 ? "+" : ""}${rd.toFixed(1)}×`.padStart(16)} |`,
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
          "  With no args: prints the FFI CPU A/B table (webui vs JS control).\n" +
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

  console.log("WebUI FFI CPU benchmark — A/B vs pure-JS control (process.cpuUsage)");
  console.log("==================================================================");
  console.log(`Addon:   ${addonPath}`);
  console.log(`Node:    ${process.version} | platform: ${process.platform}-${process.arch}`);
  console.log(`Protocol: ${protocol.length} bytes | route: "/" (dashboard)`);
  console.log("Arms measured per request, all from the SAME live object:");
  console.log("  webui (stringify+render): JSON.stringify(state) + addon.render() — true per-request cost.");
  console.log("  webui (render only):      addon.render() on a prebuilt string — hides the stringify.");
  console.log("  control (pure JS):        renderControl(state) — no stringify, no parse (the control).");

  const rows = [];
  for (const scale of SCALES) {
    const state = buildState(scale);
    const res = measureScale(addon, protocol, entry, state);
    printScale(scale, res);
    // Persist only the timed arms — host memory (res.memory) is informational
    // and GC-noisy, so it stays out of the regression snapshot.
    rows.push({ scale, stateBytes: res.stateBytes, arms: res.arms });
  }

  console.log();
  console.log("Reading the result:");
  console.log("  * webui/control ratio ~= how much more CPU the FFI path burns than plain JS.");
  console.log("  * The control render is fixed-size (5 recent cards), so its cost is flat across");
  console.log("    scales; webui grows because it re-parses the whole state every request.");
  console.log("  * `stringify only` is the JS-side tax; `webui (render only)` is the Rust-side");
  console.log("    serde parse + protocol walk. Their sum ~= `webui (stringify+render)`.");
  console.log("  * ops/s = single-thread render throughput (1e6 / wall µs/op) — the speed/throughput KPI.");
  console.log("  * CPU% = on-core saturation during the op (≈100% everywhere → compute-bound).");
  console.log(`  * core%@150 = projected % of ONE core at ${TARGET_RPS} RPS (per-req CPU × RPS) —`);
  console.log("    the number to compare against the load test's ~60% (webui) vs ~25% (control).");
  console.log("  * host memory = process RSS/heap working set under load; the per-op serde tree is");
  console.log("    transient (freed each call) so exact allocs/bytes live in `xtask bench state-cpu`.");

  if (mode === "save") saveSnapshot(name, rows);
  else if (mode === "compare") {
    const baseline = loadSnapshot(name);
    if (baseline) printDiff(rows, baseline);
  }
}

main();
