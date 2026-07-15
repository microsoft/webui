// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import * as esbuild from "esbuild";
import { esbuildProjection } from "@microsoft/webui/projection.js";
import { runWebUIClientBuild } from "../build-webui-client.mjs";

await runWebUIClientBuild(esbuild, esbuildProjection, {
  entryPoints: ["src/index.ts"],
  outfile: "dist/index.js",
  bundle: true,
  format: "esm",
  minify: !process.argv.includes("--watch"),
  sourcemap: true,
});
