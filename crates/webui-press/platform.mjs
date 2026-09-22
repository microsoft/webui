// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const __dirname = path.dirname(fileURLToPath(import.meta.url));

const PLATFORMS = {
  "darwin-arm64": "@microsoft/webui-press-darwin-arm64",
  "darwin-x64": "@microsoft/webui-press-darwin-x64",
  "linux-arm64": "@microsoft/webui-press-linux-arm64",
  "linux-x64": "@microsoft/webui-press-linux-x64",
  "win32-arm64": "@microsoft/webui-press-win32-arm64",
  "win32-x64": "@microsoft/webui-press-win32-x64",
};

export function platformKey(platform = process.platform, arch = os.arch()) {
  return `${platform}-${arch}`;
}

export function packageNameFor(platform = process.platform, arch = os.arch()) {
  const key = platformKey(platform, arch);
  const name = PLATFORMS[key];
  if (!name) {
    throw new Error(
      `[webui-press] Unsupported platform: ${key}. ` +
        `Supported: ${Object.keys(PLATFORMS).join(", ")}`,
    );
  }
  return name;
}

export function binaryNameFor(platform = process.platform) {
  return platform === "win32" ? "webui-press.exe" : "webui-press";
}

export function resolveBinary(options = {}) {
  return resolveBinaryFrom(options);
}

export function resolveBinaryFrom({
  env = process.env,
  platform = process.platform,
  arch = os.arch(),
  packageBase,
  workspaceRoot = path.resolve(__dirname, "..", ".."),
} = {}) {
  if (env.WEBUI_PRESS_BINARY_PATH) {
    return env.WEBUI_PRESS_BINARY_PATH;
  }

  const binName = binaryNameFor(platform);
  if (packageBase) {
    const binPath = path.join(packageBase, "bin", binName);
    if (existsSync(binPath)) {
      return binPath;
    }
  } else {
    try {
      const pkgDir = path.dirname(require.resolve(`${packageNameFor(platform, arch)}/package.json`));
      const binPath = path.join(pkgDir, "bin", binName);
      if (existsSync(binPath)) {
        return binPath;
      }
    } catch {
      // Fall through to workspace fallback.
    }
  }

  for (const profile of ["release", "debug"]) {
    const binPath = path.join(workspaceRoot, "target", profile, binName);
    if (existsSync(binPath)) {
      return binPath;
    }
  }

  return null;
}
