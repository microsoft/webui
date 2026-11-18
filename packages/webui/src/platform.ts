// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createRequire } from "node:module";
import { existsSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const __dirname = path.dirname(fileURLToPath(import.meta.url));

const PLATFORMS: Record<string, string> = {
  "darwin-arm64": "@microsoft/webui-darwin-arm64",
  "darwin-x64": "@microsoft/webui-darwin-x64",
  "linux-x64": "@microsoft/webui-linux-x64",
  "linux-arm64": "@microsoft/webui-linux-arm64",
  "win32-x64": "@microsoft/webui-win32-x64",
  "win32-arm64": "@microsoft/webui-win32-arm64",
};

export function platformKey(): string {
  return `${process.platform}-${os.arch()}`;
}

export function packageName(): string {
  const key = platformKey();
  const name = PLATFORMS[key];
  if (!name) {
    throw new Error(
      `[webui] Unsupported platform: ${key}. ` +
        `Supported: ${Object.keys(PLATFORMS).join(", ")}`,
    );
  }
  return name;
}

/**
 * Resolve a file from the platform-specific package, with fallback to
 * local cargo build output for workspace development.
 *
 * @param kind - CLI, runtime addon, or explicitly selected compiler addon.
 * @returns Absolute path, or null if not found.
 */
export function resolve(kind: "bin" | "addon" | "build-addon"): string | null {
  // Environment variable overrides
  if (kind === "bin" && process.env["WEBUI_BINARY_PATH"]) {
    return process.env["WEBUI_BINARY_PATH"];
  }
  if (kind === "addon" && process.env["WEBUI_ADDON_PATH"]) {
    return process.env["WEBUI_ADDON_PATH"];
  }
  if (kind === "build-addon" && process.env["WEBUI_BUILD_ADDON_PATH"]) {
    return process.env["WEBUI_BUILD_ADDON_PATH"];
  }

  // Try platform-specific npm package
  try {
    const pkg = packageName();
    const pkgDir = path.dirname(require.resolve(`${pkg}/package.json`));
    if (kind === "bin") {
      const binName = process.platform === "win32" ? "webui.exe" : "webui";
      const binPath = path.join(pkgDir, "bin", binName);
      if (existsSync(binPath)) return binPath;
    } else {
      const addonPath = path.join(pkgDir, kind === "build-addon" ? "webui-build.node" : "webui.node");
      if (existsSync(addonPath)) return addonPath;
    }
  } catch {
    // Fall through to workspace fallback
  }

  // Workspace fallback: look for cargo build output.
  // __dirname is packages/webui/dist (compiled) or packages/webui/src (source)
  // so ../../.. reaches the workspace root.
  const workspaceRoot = path.resolve(__dirname, "..", "..", "..");
  const releasePath = path.join(workspaceRoot, "target", "release");

  if (kind === "bin") {
    const binName = process.platform === "win32" ? "webui.exe" : "webui";
    const binPath = path.join(releasePath, binName);
    if (existsSync(binPath)) return binPath;
    // Also check debug
    const debugPath = path.join(workspaceRoot, "target", "debug", binName);
    if (existsSync(debugPath)) return debugPath;
  } else {
    const stem = kind === "build-addon" ? "webui_node_build" : "webui_node";
    const addonName = process.platform === "win32" ? `${stem}.dll`
      : process.platform === "darwin" ? `lib${stem}.dylib` : `lib${stem}.so`;
    const addonPath = path.join(releasePath, addonName);
    if (existsSync(addonPath)) return addonPath;
    const debugPath = path.join(workspaceRoot, "target", "debug", addonName);
    if (existsSync(debugPath)) return debugPath;
  }

  return null;
}
