// Render a WebUI protocol into dist/index.html using the napi binding.
// Expects protocol.bin to be pre-built by `webui build`.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

/**
 * Load the webui-node native addon from the build output.
 * Uses process.dlopen() to load the shared library directly.
 */
function loadAddon() {
  const root = path.resolve(__dirname, "..", "..", "..", "..");
  const profiles = ["debug", "release"];

  // napi-rs cdylib naming varies by platform
  const libNames =
    process.platform === "darwin"
      ? ["libwebui_node.dylib"]
      : process.platform === "win32"
        ? ["webui_node.dll"]
        : ["libwebui_node.so"];

  const candidates = [];
  for (const profile of profiles) {
    for (const libName of libNames) {
      candidates.push(path.join(root, "target", profile, libName));
    }
  }

  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) {
      const mod = { exports: {} };
      process.dlopen(mod, candidate);
      return mod.exports;
    }
  }

  throw new Error(
    `Could not find webui-node native addon. Looked in:\n${candidates.join("\n")}\n\nRun 'cargo build -p webui-node' first.`,
  );
}

const addon = loadAddon();

/**
 * Stream rendered HTML directly to an Express response.
 *
 * @param {object} paths - App paths { protocolBin, data }
 * @param {object} res - Express response object
 */
export function renderToResponse(paths, res) {
  const protocolData = fs.readFileSync(paths.protocolBin);
  const stateJson = fs.readFileSync(paths.data, "utf-8");

  addon.render(protocolData, stateJson, (chunk) => {
    res.write(chunk);
  });
}
