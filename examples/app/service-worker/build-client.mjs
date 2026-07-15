// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import * as esbuild from "esbuild";
import { esbuildProjection } from "@microsoft/webui/projection.js";
import { runWebUIClientBuild } from "../../build-webui-client.mjs";

await runWebUIClientBuild(esbuild, esbuildProjection, {
  entryPoints: ["src/app.ts", "src/service-worker.ts"],
  outdir: "public",
  bundle: true,
  format: "esm",
  target: "es2022",
  external: ["./wasm/handler/webui_wasm_handler.js"],
});
