// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Minimal Node.js example: uses the @microsoft/webui package to build
// templates into a protocol and render HTML with state data.
//
// Prerequisites:
//   1. Build the native addon: cargo build -p microsoft-webui-node
//   2. Build the @microsoft/webui package: pnpm --filter @microsoft/webui build
//   3. Install workspace dependencies: pnpm install
//
// Usage:
//   node index.js                                  # build + render hello-world app
//   node index.js <protocol.bin> <state.json>      # render a pre-built protocol

import { readFileSync } from "fs";
import { resolve } from "path";
import { build, render, renderStream } from "@microsoft/webui";

const appDir = resolve(import.meta.dirname, "../../app/hello-world/src");
const stateFile = resolve(import.meta.dirname, "../../app/hello-world/data/state.json");

if (process.argv[2] && process.argv[3]) {
  // Pre-built protocol mode: render an existing protocol.bin with state.json
  const protocolPath = resolve(import.meta.dirname, process.argv[2]);
  const statePath = resolve(import.meta.dirname, process.argv[3]);
  const protocolData = readFileSync(protocolPath);
  const stateJson = readFileSync(statePath, "utf-8");

  renderStream(protocolData, stateJson, (chunk) => process.stdout.write(chunk));
  process.stdout.write("\n");
} else {
  // Build + render mode (default): compile hello-world templates, then render
  const state = JSON.parse(readFileSync(stateFile, "utf-8"));
  const result = build({ appDir });

  console.error(
    `Built: ${result.stats.fragmentCount} fragments, ` +
    `${result.stats.protocolSizeBytes} bytes protocol, ` +
    `${result.stats.durationMs}ms`
  );

  const html = render(result.protocol, state);
  process.stdout.write(html);
  process.stdout.write("\n");
}
