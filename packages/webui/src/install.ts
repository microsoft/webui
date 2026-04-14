// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { resolve, platformKey, packageName } from "./platform.js";

// dist/install.js lives one level below the package root, so bin/ is at ../bin.
const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Find the platform-specific Rust binary.
let srcBinPath: string | null = null;
try {
  srcBinPath = resolve("bin");
} catch {
  // Fall through to warning.
}

if (srcBinPath && fs.existsSync(srcBinPath)) {
  // On non-Windows, replace bin/webui (the JS shim placeholder) with the
  // actual Rust binary so the CLI runs directly without a Node.js wrapper.
  //
  // On Windows, keep the shim: npm/pnpm generate a .cmd wrapper that calls
  // `node` on the bin entry, so placing a raw .exe there would not work.
  // The shim resolves and spawns webui.exe at runtime instead.
  if (process.platform !== "win32") {
    const binDir = path.join(__dirname, "..", "bin");
    const destPath = path.join(binDir, "webui");
    try {
      fs.mkdirSync(binDir, { recursive: true });
      fs.copyFileSync(srcBinPath, destPath);
      fs.chmodSync(destPath, 0o755);
    } catch {
      // Copy failed — the JS shim fallback remains in place.
    }
  }
  process.exit(0);
}

// Binary not found — emit a warning.
const key = platformKey();
let pkg: string | undefined;
try {
  pkg = packageName();
} catch {
  console.warn(
    `[webui] Warning: Unsupported platform ${key}. ` +
      `The webui CLI will not be available. ` +
      `Set WEBUI_BINARY_PATH to use a custom binary.`,
  );
  process.exit(0);
}

console.warn(
  `[webui] Warning: Platform package ${pkg!} was not installed. ` +
    `This usually means your package manager was run with --no-optional. ` +
    `The webui CLI and native addon will not be available.\n` +
    `To fix: reinstall without --no-optional, or set WEBUI_BINARY_PATH.`,
);
