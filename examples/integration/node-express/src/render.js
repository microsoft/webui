// Render a WebUI app template into dist/index.html using the napi binding.

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
 * Render a WebUI template + state to dist/index.html.
 *
 * @param {object} paths - App paths { template, data, distDir }
 */
export function renderToIndexHtml(paths) {
  const template = fs.readFileSync(paths.template, "utf-8");
  const stateJson = fs.readFileSync(paths.data, "utf-8");

  const html = addon.render(template, stateJson);

  fs.mkdirSync(paths.distDir, { recursive: true });
  fs.writeFileSync(path.join(paths.distDir, "index.html"), html, "utf-8");
}
