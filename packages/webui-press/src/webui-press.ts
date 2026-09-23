#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { spawnSync } from "node:child_process";
import { packageNameFor, platformKey, resolveBinary } from "./platform.js";

const binary = resolveBinary();

if (!binary) {
  const key = platformKey();
  try {
    const pkg = packageNameFor();
    console.error(
      `[webui-press] Platform package ${pkg} was not installed. ` +
        `Reinstall without --no-optional, or set WEBUI_PRESS_BINARY_PATH.`,
    );
  } catch {
    console.error(
      `[webui-press] Unsupported platform ${key}. ` +
        `Set WEBUI_PRESS_BINARY_PATH to use a custom binary.`,
    );
  }
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(
    `[webui-press] Failed to run ${binary}: ${
      result.error instanceof Error ? result.error.message : String(result.error)
    }`,
  );
  process.exit(1);
}

if (result.signal) {
  process.kill(process.pid, result.signal);
} else {
  process.exit(result.status ?? 1);
}
