// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createRequire } from "node:module";
import { packageName, platformKey, resolve } from "./platform.js";

const require = createRequire(import.meta.url);
type AddonKind = "addon" | "build-addon";
const addons: Partial<Record<AddonKind, unknown>> = {};

export function loadBinding<T>(kind: AddonKind = "addon"): T {
  const cached = addons[kind];
  if (cached) return cached as T;
  const addonPath = resolve(kind);
  const label = kind === "build-addon" ? "build addon" : "addon";
  if (!addonPath) {
    throw new Error(
      `[webui] Native ${label} not found for ${platformKey()}. ${addonInstallHelp(kind)} ` +
      'The Node API does not fall back to the CLI; invoke "webui" explicitly for filesystem builds.',
    );
  }
  let addon: unknown;
  try {
    if (addonPath.endsWith(".node")) {
      addon = require(addonPath);
    } else {
      const module: { exports: unknown } = { exports: {} };
      process.dlopen(module, addonPath);
      addon = module.exports;
    }
  } catch (cause) {
    throw new Error(
      `[webui] Failed to load native ${label} at ${addonPath}. ${addonInstallHelp(kind)}`,
      { cause },
    );
  }
  addons[kind] = addon;
  return addon as T;
}

export function addonInstallHelp(kind: AddonKind = "addon"): string {
  const override = kind === "build-addon" ? "WEBUI_BUILD_ADDON_PATH" : "WEBUI_ADDON_PATH";
  try {
    return `Reinstall ${packageName()} or set ${override} to a compatible addon.`;
  } catch (error) {
    const platformError = error instanceof Error ? error.message : String(error);
    return `${platformError} Set ${override} to a compatible addon.`;
  }
}
