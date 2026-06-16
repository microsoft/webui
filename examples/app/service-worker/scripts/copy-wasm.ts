// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { access, copyFile, mkdir, rm } from "fs/promises";
import { spawnSync, type SpawnSyncReturns } from "node:child_process";
import { dirname, resolve } from "path";
import { fileURLToPath } from "url";

const here = dirname(fileURLToPath(import.meta.url));
const exampleRoot = resolve(here, "..");
const repoRoot = resolve(exampleRoot, "../../..");
const sourceDir = resolve(repoRoot, "docs/.webui-press/public/wasm/handler");
const destDir = resolve(exampleRoot, "public/wasm/handler");
const runtimeFiles = [
  "webui_wasm_handler.js",
  "webui_wasm_handler_bg.wasm",
  "webui_wasm_handler.d.ts",
  "webui_wasm_handler_bg.wasm.d.ts",
];

await main();

async function main(): Promise<void> {
  await ensureHandlerWasm();
  await rm(destDir, { recursive: true, force: true });
  await mkdir(destDir, { recursive: true });
  for (const file of runtimeFiles) {
    await copyFile(resolve(sourceDir, file), resolve(destDir, file));
  }

  console.log(`Copied handler WASM bundle to ${destDir}`);
}

async function ensureHandlerWasm(): Promise<void> {
  try {
    await access(resolve(sourceDir, "webui_wasm_handler.js"));
    await access(resolve(sourceDir, "webui_wasm_handler_bg.wasm"));
    return;
  } catch {
    const result: SpawnSyncReturns<Buffer> = spawnSync(
      "cargo",
      ["xtask", "build-wasm"],
      {
        cwd: repoRoot,
        stdio: "inherit",
      },
    );
    if (result.status !== 0) {
      throw new Error("cargo xtask build-wasm failed");
    }
  }
}
