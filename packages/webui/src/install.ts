// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import fs from "node:fs";
import { resolve, platformKey, packageName } from "./platform.js";

// The package.json "bin" entry is a JavaScript launcher. Do not copy a
// host-native binary into this package during publish; just verify and repair
// the platform package binary when lifecycle scripts are enabled.
try {
  if (process.env["WEBUI_BINARY_PATH"]) {
    process.exit(0);
  }

  const srcBin = resolve("bin");
  if (srcBin && fs.existsSync(srcBin)) {
    fs.chmodSync(srcBin, 0o755);
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
