// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Minimal Node.js example: load a pre-built protocol.bin, pass state JSON,
// and print rendered HTML to stdout using the @microsoft/webui package.
//
// Prerequisites:
//   1. Build the native addon: cargo build -p microsoft-webui-node
//   2. Build the hello-world app:
//      cargo run -p microsoft-webui-cli -- build ../../app/hello-world/templates --out ../../app/hello-world/dist
//
// Usage:
//   node index.js [protocol.bin] [state.json] [--plugin=fast]

import { readFileSync } from "fs";
import { resolve } from "path";
import { renderStream } from "@microsoft/webui";

if (!process.argv[2] || !process.argv[3]) {
  console.error("Usage: node index.js <protocol.bin> <state.json> [--plugin=fast]");
  console.error("  protocol.bin  Path to the compiled protocol binary");
  console.error("  state.json    Path to the JSON state file");
  process.exit(1);
}

const protocolPath = process.argv[2];
const statePath = process.argv[3];

// Check for --plugin=fast flag
const pluginArg = process.argv.find((a) => a.startsWith("--plugin="));
const pluginName = pluginArg ? pluginArg.split("=")[1] : undefined;

const protocolData = readFileSync(resolve(import.meta.dirname, protocolPath));
const stateJson = readFileSync(
  resolve(import.meta.dirname, statePath),
  "utf-8"
);

// Render, streaming each chunk to stdout
renderStream(protocolData, stateJson, (chunk) => process.stdout.write(chunk), pluginName);
process.stdout.write("\n");
