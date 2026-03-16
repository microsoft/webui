// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Minimal Node.js example: load a pre-built protocol.bin, pass state JSON,
// and print rendered HTML to stdout using the webui-node native addon.
//
// Prerequisites:
//   1. Build the native addon: cargo build -p webui-node
//   2. Build the hello-world app:
//      cargo run -p webui-cli -- build ../../app/hello-world/templates --out ../../app/hello-world/dist
//
// Usage:
//   node index.js [protocol.bin] [state.json] [--plugin=fast]

import { readFileSync } from "fs";
import { createRequire } from "module";
import { resolve, join } from "path";
import { platform } from "os";

// Resolve the native addon from the cargo build output
function loadAddon() {
  const require = createRequire(import.meta.url);
  const root = resolve(import.meta.dirname, "../../..");
  const profiles = ["debug", "release"];
  const ext =
    platform() === "darwin"
      ? "dylib"
      : platform() === "win32"
        ? "dll"
        : "so";
  const prefix = platform() === "win32" ? "" : "lib";
  const filename = `${prefix}webui_node.${ext}`;

  for (const profile of profiles) {
    try {
      return require(join(root, "target", profile, filename));
    } catch {
      // try next profile
    }
  }
  throw new Error(
    `Could not find ${filename} in target/debug or target/release. Run: cargo build -p webui-node`
  );
}

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

const addon = loadAddon();
const protocolData = readFileSync(resolve(import.meta.dirname, protocolPath));
const stateJson = readFileSync(
  resolve(import.meta.dirname, statePath),
  "utf-8"
);

// Pass plugin name as 4th arg to the native addon
const handlerPlugin = pluginName === "fast" ? "fast" : undefined;

// Render, streaming each chunk to stdout
addon.render(protocolData, stateJson, (chunk) => process.stdout.write(chunk), handlerPlugin);
process.stdout.write("\n");
