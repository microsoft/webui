// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createRequire } from "node:module";
import { packageName, platformKey, resolve } from "./platform.js";

const require = createRequire(import.meta.url);
let addon: unknown;

export function loadBinding<T>(): T {
  if (addon) return addon as T;
  const addonPath = resolve("addon");
  if (!addonPath) {
    throw new Error(
      `[webui] Native addon not found for ${platformKey()}. ${addonInstallHelp()} ` +
      'The Node API does not fall back to the CLI; invoke "webui" explicitly for filesystem builds.',
    );
  }
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
      `[webui] Failed to load native addon at ${addonPath}. ${addonInstallHelp()}`,
      { cause },
    );
  }
  return addon as T;
}

export function addonInstallHelp(): string {
  try {
    return `Reinstall ${packageName()} or set WEBUI_ADDON_PATH to a compatible addon.`;
  } catch (error) {
    const platformError = error instanceof Error ? error.message : String(error);
    return `${platformError} Set WEBUI_ADDON_PATH to a compatible addon.`;
  }
}
