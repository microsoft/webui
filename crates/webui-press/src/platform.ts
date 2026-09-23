// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PACKAGE_ROOT = path.resolve(__dirname, "..");

const PLATFORMS: Record<string, string> = {
  "darwin-arm64": "@microsoft/webui-press-darwin-arm64",
  "darwin-x64": "@microsoft/webui-press-darwin-x64",
  "linux-arm64": "@microsoft/webui-press-linux-arm64",
  "linux-x64": "@microsoft/webui-press-linux-x64",
  "win32-arm64": "@microsoft/webui-press-win32-arm64",
  "win32-x64": "@microsoft/webui-press-win32-x64",
};

export interface ResolveBinaryOptions {
  env?: NodeJS.ProcessEnv;
  platform?: NodeJS.Platform | string;
  arch?: string;
  packageBase?: string | null;
  workspaceRoot?: string;
}

export function platformKey(
  platform: NodeJS.Platform | string = process.platform,
  arch: string = os.arch(),
): string {
  return `${platform}-${arch}`;
}

export function packageNameFor(
  platform: NodeJS.Platform | string = process.platform,
  arch: string = os.arch(),
): string {
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

export function binaryNameFor(platform: NodeJS.Platform | string = process.platform): string {
  return platform === "win32" ? "webui-press.exe" : "webui-press";
}

export function workspaceRootForPackage(packageRoot: string = PACKAGE_ROOT): string {
  return path.resolve(packageRoot, "..", "..");
}

export function resolveBinary(options?: ResolveBinaryOptions): string | null {
  return resolveBinaryFrom(options);
}

export function resolveBinaryFrom({
  env = process.env,
  platform = process.platform,
  arch = os.arch(),
  packageBase,
  workspaceRoot = workspaceRootForPackage(),
}: ResolveBinaryOptions = {}): string | null {
  if (env.WEBUI_PRESS_BINARY_PATH) {
    return env.WEBUI_PRESS_BINARY_PATH;
  }

  const binName = binaryNameFor(platform);
  if (packageBase !== null) {
    const localBin = path.join(packageBase ?? PACKAGE_ROOT, "bin", binName);
    if (existsSync(localBin)) {
      return localBin;
    }
  }

  try {
    const pkgDir = path.dirname(require.resolve(`${packageNameFor(platform, arch)}/package.json`));
    const platformBin = path.join(pkgDir, "bin", binName);
    if (existsSync(platformBin)) {
      return platformBin;
    }
  } catch {
    // Fall through to workspace fallback.
  }

  for (const profile of ["release", "debug"]) {
    const workspaceBin = path.join(workspaceRoot, "target", profile, binName);
    if (existsSync(workspaceBin)) {
      return workspaceBin;
    }
  }

  return null;
}
