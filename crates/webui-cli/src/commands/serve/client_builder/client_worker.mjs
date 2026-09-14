// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// @ts-check

import { Console } from "node:console";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

/** @typedef {{ rebuild(): Promise<unknown>, dispose(): Promise<void>, watchPaths?: string[] }} Builder */
/** @typedef {{ type: "ready", watchPaths: string[] } | { type: "built" } | { type: "error", message: string } | { type: "stopped" }} Reply */

const MAX_RECORD = 64 * 1024;
const SHUTDOWN_GRACE_MS = 2000;
const write = process.stdout.write.bind(process.stdout);
process.stdout.write = process.stderr.write.bind(process.stderr);
const diagnostics = new Console({ stdout: process.stderr, stderr: process.stderr });

/** @type {Builder | undefined} */
let builder;
let busy = true;
let stopping = false;
/** @type {NodeJS.Timeout | undefined} */
let shutdownTimer;
/** @type {() => void} */
let initialized;
/** @type {Promise<void>} */
const initialization = new Promise((resolve) => { initialized = resolve; });
const input = Buffer.alloc(MAX_RECORD);
let buffered = 0;
const decoder = new TextDecoder("utf-8", { fatal: true });

/** @param {unknown} error */
function message(error) {
  return error instanceof Error ? error.stack || error.message : String(error);
}

/** @param {Reply} record */
async function send(record) {
  // Cap source text before JSON escaping can allocate a serialized copy.
  if (record.type === "error" && record.message.length > MAX_RECORD) {
    throw new Error("Client builder error exceeds 64 KiB; shorten error diagnostics.");
  }
  const line = JSON.stringify(record) + "\n";
  if (Buffer.byteLength(line) > MAX_RECORD) {
    throw new Error("Client builder reply exceeds 64 KiB; shorten watchPaths or error diagnostics.");
  }
  await new Promise((resolve, reject) => {
    write(line, (error) => error ? reject(error) : resolve(undefined));
  });
}

function boundShutdown() {
  if (shutdownTimer) return;
  // EOF can arrive while initialization/rebuild is pending. This timer is
  // deliberately referenced so an unresolved hook cannot strand this process.
  shutdownTimer = setTimeout(() => {
    diagnostics.error("Client builder shutdown timed out; fix hung initialization/dispose hooks.");
    process.exit(1);
  }, SHUTDOWN_GRACE_MS);
}

/** @param {boolean} acknowledge @param {number} code */
async function shutdown(acknowledge, code) {
  // Explicit stop is bounded by the native caller's configured timeout.
  // EOF must also bound an explicit stop whose caller subsequently disappeared.
  if (!acknowledge) boundShutdown();
  if (stopping) return;
  stopping = true;
  try {
    await initialization;
    if (typeof builder?.dispose === "function") await builder.dispose();
    if (acknowledge) await send({ type: "stopped" });
  } catch (error) {
    diagnostics.error("Client builder disposal failed:", error);
    code = 1;
  }
  process.exit(code);
}

/** @param {unknown} error */
function fatal(error) {
  diagnostics.error(
    "Client builder runtime failed; check the module and restart the server:",
    error,
  );
  void shutdown(false, 1);
}

async function rebuild() {
  try {
    /** @type {Reply} */
    let reply;
    try {
      if (!builder) throw new Error("Client builder is not initialized.");
      await builder.rebuild();
      reply = { type: "built" };
    } catch (error) {
      reply = { type: "error", message: message(error) || "Client build rejected without a diagnostic." };
    }
    if (!stopping) await send(reply);
    busy = false;
  } catch (error) {
    fatal(error);
  }
}

/** @param {Uint8Array} line */
function command(line) {
  const record = JSON.parse(decoder.decode(line));
  if (!record || typeof record !== "object" || Array.isArray(record)
      || Object.keys(record).length !== 1
      || (record.type !== "build" && record.type !== "stop")) {
    throw new Error("Invalid private client builder command.");
  }
  if (busy || stopping) throw new Error("Overlapping private client builder requests.");
  busy = true;
  if (record.type === "stop") void shutdown(true, 0);
  else void rebuild();
}

/** @param {Buffer} chunk */
function consume(chunk) {
  let start = 0;
  while (start < chunk.length) {
    const newline = chunk.indexOf(10, start);
    const end = newline === -1 ? chunk.length : newline + 1;
    const length = end - start;
    if (length > MAX_RECORD - buffered) {
      throw new Error("Private client builder command exceeds 64 KiB.");
    }
    chunk.copy(input, buffered, start, end);
    buffered += length;
    if (newline !== -1) {
      command(input.subarray(0, buffered));
      buffered = 0;
    }
    start = end;
  }
}

process.stdin.on("data", (chunk) => {
  if (stopping) return;
  try { consume(chunk); } catch (error) { fatal(error); }
});
process.stdin.on("end", () => {
  if (buffered) fatal(new Error("Truncated private client builder command."));
  else void shutdown(false, 0);
});
process.stdin.on("error", fatal);
process.stdout.on("error", fatal);
process.on("uncaughtException", fatal);
process.on("unhandledRejection", fatal);

/** @param {unknown} extra @param {string} appDir */
function resolveWatchPaths(extra, appDir) {
  if (extra === undefined) return [];
  if (!Array.isArray(extra)) {
    throw new Error("Client builder watchPaths must be an array of file/directory paths.");
  }
  /** @type {string[]} */
  const paths = [];
  let size = 0;
  for (const path of extra) {
    if (typeof path !== "string" || !path.length || path.includes("\0")) {
      throw new Error("Client builder watchPaths must contain nonempty file/directory paths.");
    }
    if (path.length > MAX_RECORD) throw new Error("Client builder watchPaths exceed 64 KiB.");
    const absolute = resolve(appDir, path);
    size += absolute.length + 3;
    if (size > MAX_RECORD) throw new Error("Client builder watchPaths exceed 64 KiB.");
    paths.push(absolute);
  }
  return paths;
}

/** @param {string} module @param {string} appDir @param {string} outDir @param {string | undefined} entry */
async function createBuilder(module, appDir, outDir, entry) {
  if (module === "--builtin") {
    if (!entry) throw new Error("The builtin client builder requires a client entry.");
    return createBuiltinBuilder({ appDir, outDir, clientEntry: resolve(appDir, entry) });
  }
  const factory = (await import(pathToFileURL(resolve(module)).href)).default;
  if (typeof factory !== "function") {
    throw new Error("Client builder must export a default async factory({ appDir, outDir }).");
  }
  return factory({ appDir, outDir });
}

async function initialize() {
  try {
    const [module, application, output, entry] = process.argv.slice(1);
    if (!module || !application || !output) {
      throw new Error("Expected a client builder module, appDir, and outDir.");
    }
    const appDir = resolve(application);
    const outDir = resolve(output);
    builder = await createBuilder(module, appDir, outDir, entry);
    if (!builder || typeof builder !== "object"
        || typeof builder.rebuild !== "function" || typeof builder.dispose !== "function") {
      throw new Error("Client builder factory must return { rebuild(), dispose(), watchPaths? }.");
    }
    const watchPaths = resolveWatchPaths(builder.watchPaths, appDir);
    // Returned resources can be disposed even if readiness cannot be delivered.
    initialized();
    if (!stopping) await send({ type: "ready", watchPaths });
    busy = false;
  } catch (error) {
    fatal(error);
  } finally {
    initialized();
  }
}

void initialize();
