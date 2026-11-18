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

function identicalCli(source: string, destination: string): boolean {
  if (!fs.existsSync(destination) ||
      fs.statSync(source).size !== fs.statSync(destination).size) return false;
  const sourceFd = fs.openSync(source, "r");
  try {
    const destinationFd = fs.openSync(destination, "r");
    try {
      // Keep concurrent postbuild comparisons bounded even for debug binaries.
      const sourceBytes = Buffer.alloc(65536);
      const destinationBytes = Buffer.alloc(65536);
      for (;;) {
        const sourceRead = fs.readSync(sourceFd, sourceBytes);
        const destinationRead = fs.readSync(destinationFd, destinationBytes);
        if (sourceRead !== destinationRead ||
            !sourceBytes.subarray(0, sourceRead).equals(destinationBytes.subarray(0, destinationRead))) {
          return false;
        }
        if (sourceRead === 0) return true;
      }
    } finally {
      fs.closeSync(destinationFd);
    }
  } finally {
    fs.closeSync(sourceFd);
  }
}

function copyCli(source: string, destination: string): void {
  // Parallel workspace builds rerun postbuild after the initial shared install.
  // Do not overwrite an identical executable that another app may be running.
  if (!identicalCli(source, destination)) fs.copyFileSync(source, destination);
  fs.chmodSync(destination, 0o755);
}

try {
  const srcBin = resolve("bin");
  if (srcBin && fs.existsSync(srcBin)) {
    fs.mkdirSync(binDir, { recursive: true });
    copyCli(srcBin, npmBinDest);
    if (nativeBinDest !== npmBinDest) {
      copyCli(srcBin, nativeBinDest);
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
