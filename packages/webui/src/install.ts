// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { resolve, platformKey, packageName } from "./platform.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const binDir = path.resolve(__dirname, "..", "bin");
const npmBinDest = path.join(binDir, "webui");
const nativeBinDest = path.join(binDir, process.platform === "win32" ? "webui.exe" : "webui");

try {
  const srcBin = resolve("bin");
  if (srcBin && fs.existsSync(srcBin)) {
    fs.mkdirSync(binDir, { recursive: true });
    fs.copyFileSync(srcBin, npmBinDest);
    fs.chmodSync(npmBinDest, 0o755);
    if (nativeBinDest !== npmBinDest) {
      fs.copyFileSync(srcBin, nativeBinDest);
      fs.chmodSync(nativeBinDest, 0o755);
    }
    process.exit(0);
  }
} catch (error) {
  console.warn(`[webui] Failed to install the native CLI: ${error instanceof Error ? error.message : String(error)}`);
}

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
