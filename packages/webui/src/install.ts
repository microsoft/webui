// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import fs from "node:fs";
import { resolve, platformKey, packageName } from "./platform.js";

// Validate that the platform binary exists after install.
try {
  const binPath = resolve("bin");
  if (binPath && fs.existsSync(binPath)) {
    // Success — binary is available.
    process.exit(0);
  }
} catch {
  // Fall through to warning.
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
