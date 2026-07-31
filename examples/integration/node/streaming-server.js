// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// In-process Node.js streaming server: drives a WebUI progressive response
// from an ordinary `node:http` handler.
//
// The point of this example is the ownership split. `protocol.streamResponse()`
// returns a session whose methods *return* the bytes they produced instead of
// writing them, so this file keeps the socket, the write order, and the
// backpressure contract. WebUI never touches the transport, which is what makes
// it drop into an existing Node service.
//
// Prerequisites:
//   1. Build the native addon:  cargo build -p microsoft-webui-node
//   2. Build the packages:      pnpm --filter @microsoft/webui --filter @microsoft/webui-framework run build
//   3. Install dependencies:    pnpm install
//
// Usage:
//   node streaming-server.js [--port 3040] [--batch-delay-ms 600] [--job-delay-ms 1500]

import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { build, Protocol } from "@microsoft/webui";

const options = parseOptions(process.argv.slice(2));

// The framework's compiled ESM is served straight from its package: this
// example has no bundler, and the browser can load those modules as-is.
const frameworkDir = dirname(fileURLToPath(import.meta.resolve("@microsoft/webui-framework")));
const appDir = resolve(import.meta.dirname, "streaming-app");

// Compile once at startup. The protocol is immutable and shared by every
// request; only the per-request state below varies.
const result = build({ appDir, plugin: "webui", css: "style" });
const protocol = new Protocol(result.protocol, { plugin: "webui" });

const LOG_BATCHES = [
  [
    { level: "info", text: "resolving workspace" },
    { level: "info", text: "restoring 412 packages" },
  ],
  [
    { level: "info", text: "compiling 87 modules" },
    { level: "warn", text: "unused import in src/legacy.ts" },
  ],
  [
    { level: "info", text: "bundling client entry" },
    { level: "info", text: "done in 4.1s" },
  ],
];

const server = createServer((request, response) => {
  void handleRequest(request, response).catch((error) => {
    reportFailure(response, error);
  });
});

server.listen(options.port, "127.0.0.1", () => {
  console.log(`In-process streaming server on http://127.0.0.1:${options.port}`);
});

async function handleRequest(request, response) {
  const url = new URL(request.url ?? "/", "http://127.0.0.1");

  if (request.method !== "GET") {
    sendText(response, 405, "Method Not Allowed");
    return;
  }
  if (url.pathname === "/") {
    await streamPage(response);
    return;
  }
  if (url.pathname.startsWith("/framework/")) {
    await sendFrameworkModule(response, url.pathname.slice("/framework/".length));
    return;
  }
  sendText(response, 404, "Not Found");
}

/**
 * Render one progressive response, one chunk per call.
 *
 * Ordering is enforced by the session, so the shape below is the contract:
 * shell first, then every boundary in declaration order, updates only to
 * boundaries committed as `updatable`, and `finish()` last.
 */
async function streamPage(response) {
  const session = protocol.streamResponse({ entry: "index.html", requestPath: "/" });

  // Names are authored strings; resolve them once, outside the write loop.
  const jobStatus = session.boundary("job-status");
  const logBatches = [
    session.boundary("log-batch-1"),
    session.boundary("log-batch-2"),
    session.boundary("log-batch-3"),
  ];

  response.writeHead(200, {
    "Content-Type": "text/html; charset=utf-8",
    "Cache-Control": "no-store",
    // Reverse proxies buffer by default, which would defeat the whole point.
    "X-Accel-Buffering": "no",
  });

  // The shell carries the state the document prefix needs; each boundary below
  // carries only its own.
  await write(response, session.writeShell({ jobState: "running", jobDetail: "" }));

  // Committed before its data exists, so nothing waits on the slow job.
  await write(
    response,
    session.writeBoundary(jobStatus, { jobState: "running", jobDetail: "starting" }, "updatable"),
  );

  // Started, not awaited: the job races the log batches below.
  const job = runSlowJob(options.jobDelayMs);
  let jobPending = true;

  for (let index = 0; index < LOG_BATCHES.length; index++) {
    const batch = delay(options.batchDelayMs).then(() => "batch");

    if (jobPending) {
      const ready = await Promise.race([job, batch]);
      if (ready !== "batch") {
        // The job won: patch the already-committed boundary on this same
        // response. No second request, no DOM replacement, no re-hydration.
        await write(response, session.update(jobStatus, ready));
        jobPending = false;
        await batch;
      }
    } else {
      await batch;
    }

    await write(
      response,
      session.writeBoundary(logBatches[index], { [`batch${index + 1}`]: LOG_BATCHES[index] }),
    );
  }

  if (jobPending) {
    await write(response, session.update(jobStatus, await job));
  }

  response.end(session.finish({}));
}

/**
 * Write one chunk and honour Node's backpressure contract.
 *
 * This is the entire transport half of the integration: the session hands back
 * bytes, and the host decides when the socket is ready for more.
 *
 * A client that aborts while the socket buffer is full never emits `drain`, and
 * an aborted request surfaces as `close` rather than `error`, so waiting on
 * `drain` alone would suspend this function forever and retain the session.
 */
function write(response, chunk) {
  if (response.destroyed || response.writableEnded) {
    return Promise.reject(new Error("client disconnected before the next chunk"));
  }
  if (response.write(chunk)) {
    return Promise.resolve();
  }
  return new Promise((resolveWrite, rejectWrite) => {
    const settle = (error) => {
      response.off("drain", onDrain);
      response.off("close", onClose);
      if (error) {
        rejectWrite(error);
      } else {
        resolveWrite();
      }
    };
    const onDrain = () => settle();
    const onClose = () => settle(new Error("client disconnected while backpressured"));
    response.once("drain", onDrain);
    response.once("close", onClose);
  });
}

async function runSlowJob(delayMs) {
  await delay(delayMs);
  return { jobState: "succeeded", jobDetail: "3 targets built" };
}

async function sendFrameworkModule(response, relativePath) {
  // Contain the served path to the framework's own output directory. The
  // trailing separator matters: a bare prefix test would also accept a sibling
  // directory whose name merely starts with the framework directory's name.
  const target = join(frameworkDir, normalize(relativePath));
  if (!target.startsWith(frameworkDir + sep) || !target.endsWith(".js")) {
    sendText(response, 404, "Not Found");
    return;
  }

  try {
    await stat(target);
  } catch {
    sendText(response, 404, "Not Found");
    return;
  }

  response.writeHead(200, {
    "Content-Type": "text/javascript; charset=utf-8",
    "Cache-Control": "no-store",
  });
  createReadStream(target).pipe(response);
}

function delay(milliseconds) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}

function sendText(response, status, body) {
  response.writeHead(status, { "Content-Type": "text/plain; charset=utf-8" });
  response.end(body);
}

function reportFailure(response, error) {
  const message = error instanceof Error ? error.message : String(error);
  console.error(`streaming request failed: ${message}`);
  if (response.destroyed) {
    return;
  }
  // Once the shell is on the wire the status line is already committed, so a
  // late failure can only be signalled by dropping the connection.
  if (response.headersSent) {
    response.destroy();
  } else {
    sendText(response, 500, "Internal Server Error");
  }
}

function parseOptions(args) {
  return {
    port: numberOption(args, "--port", 3040),
    batchDelayMs: numberOption(args, "--batch-delay-ms", 600),
    jobDelayMs: numberOption(args, "--job-delay-ms", 1500),
  };
}

function numberOption(args, name, fallback) {
  const index = args.indexOf(name);
  if (index === -1) {
    return fallback;
  }
  const value = Number(args[index + 1]);
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${name} requires a non-negative integer`);
  }
  return value;
}
