// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import * as esbuild from "esbuild";
import { esbuildProjection } from "@microsoft/webui/projection.js";
import { runWebUIClientBuild } from "../../build-webui-client.mjs";

const builds = [];
builds.push(await runWebUIClientBuild(esbuild, esbuildProjection, {
  entryPoints: ["src/index.ts"],
  outdir: "dist",
  bundle: true,
  format: "esm",
  splitting: true,
  chunkNames: "chunks/[name]-[hash]",
  minify: !process.argv.includes("--watch"),
  sourcemap: process.argv.includes("--watch"),
}));
builds.push(await runWebUIClientBuild(esbuild, esbuildProjection, {
  entryPoints: [
    "external-components/external-panel/external-panel.ts",
  ],
  outfile: "dist/external/external-panel.js",
  bundle: true,
  format: "esm",
  minify: !process.argv.includes("--watch"),
  sourcemap: process.argv.includes("--watch"),
  projectionManifest:
    "dist/external/webui-projection.json",
}));
