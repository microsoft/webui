// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { binaryNameFor, packageNameFor, platformKey, resolveBinary } from "./platform.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const binDir = path.join(__dirname, "bin");
const npmBinDest = path.join(binDir, "webui-press");
const nativeBinDest = path.join(binDir, binaryNameFor());

try {
  const srcBin = resolveBinary();
  if (srcBin && fs.existsSync(srcBin)) {
    fs.mkdirSync(binDir, { recursive: true });
    if (path.resolve(srcBin) !== path.resolve(npmBinDest)) {
      fs.copyFileSync(srcBin, npmBinDest);
    }
    fs.chmodSync(npmBinDest, 0o755);
    if (nativeBinDest !== npmBinDest) {
      if (path.resolve(srcBin) !== path.resolve(nativeBinDest)) {
        fs.copyFileSync(srcBin, nativeBinDest);
      }
      fs.chmodSync(nativeBinDest, 0o755);
    }
    process.exit(0);
  }
} catch (error) {
  console.warn(
    `[webui-press] Warning: Failed to copy the native binary: ${
      error instanceof Error ? error.message : String(error)
    }`,
  );
  // Fall through to package guidance.
}

if (
  process.env.npm_lifecycle_event === "postinstall" &&
  fs.existsSync(path.join(__dirname, "Cargo.toml"))
) {
  process.exit(0);
}

const key = platformKey();
try {
  const pkg = packageNameFor();
  console.warn(
    `[webui-press] Warning: Platform package ${pkg} was not installed. ` +
      `This usually means your package manager was run with --no-optional. ` +
      `The webui-press CLI will not be available.\n` +
      `To fix: reinstall without --no-optional, or set WEBUI_PRESS_BINARY_PATH.`,
  );
} catch {
  console.warn(
    `[webui-press] Warning: Unsupported platform ${key}. ` +
      `The webui-press CLI will not be available. ` +
      `Set WEBUI_PRESS_BINARY_PATH to use a custom binary.`,
  );
}
