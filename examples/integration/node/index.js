// Minimal Node.js example: load a pre-built protocol.bin, pass state JSON,
// and print rendered HTML to stdout using the webui-node native addon.
//
// Prerequisites:
//   1. Build the native addon: cargo build -p webui-node
//   2. Build the hello-world app:
//      cargo run -p webui-cli -- build ../../app/hello-world/templates --out ../../app/hello-world/dist
//
// Usage:
//   node index.js [protocol.bin] [state.json]

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

const protocolPath =
  process.argv[2] || "../../app/hello-world/dist/protocol.bin";
const statePath = process.argv[3] || "../../app/hello-world/data/state.json";

const addon = loadAddon();
const protocolData = readFileSync(resolve(import.meta.dirname, protocolPath));
const stateJson = readFileSync(
  resolve(import.meta.dirname, statePath),
  "utf-8"
);

const html = addon.render(protocolData, stateJson);
process.stdout.write(html + "\n");
