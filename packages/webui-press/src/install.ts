// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  binaryNameFor,
  packageNameFor,
  platformKey,
  resolveBinary,
  workspaceRootForPackage,
} from "./platform.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(__dirname, "..");
const binDir = path.join(packageRoot, "bin");
const npmBinDest = path.join(binDir, "webui-press");
const nativeBinDest = path.join(binDir, binaryNameFor());
const workspaceRoot = workspaceRootForPackage(packageRoot);
const workspaceCargoToml = path.join(workspaceRoot, "Cargo.toml");

function installResolvedBinary(): boolean {
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
    return true;
  }
  return false;
}

try {
  if (installResolvedBinary()) {
    process.exit(0);
  }
} catch (error) {
  console.warn(
    `[webui-press] Warning: Failed to copy the native binary: ${
      error instanceof Error ? error.message : String(error)
    }`,
  );
}

if (process.env.npm_lifecycle_event === "build" && fs.existsSync(workspaceCargoToml)) {
  const result = spawnSync("cargo", ["build", "-p", "microsoft-webui-press"], {
    cwd: workspaceRoot,
    stdio: "inherit",
  });
  if (result.error) {
    console.error(
      `[webui-press] Failed to build the workspace binary: ${
        result.error instanceof Error ? result.error.message : String(result.error)
      }`,
    );
    process.exit(1);
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
  try {
    if (installResolvedBinary()) {
      process.exit(0);
    }
  } catch (error) {
    console.warn(
      `[webui-press] Warning: Failed to copy the workspace binary: ${
        error instanceof Error ? error.message : String(error)
      }`,
    );
  }
}

if (process.env.npm_lifecycle_event === "postinstall" && fs.existsSync(workspaceCargoToml)) {
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
      `The webui-press CLI will not be available.\n` +
      `Set WEBUI_PRESS_BINARY_PATH to use a custom binary.`,
  );
}
