// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { runWebUIClientBuild } from "../../build-client.mjs";

const builds = [];
builds.push(await runWebUIClientBuild({
  chunkNames: "chunks/[name]-[hash]",
}));
builds.push(await runWebUIClientBuild({
  entryPoints: [
    "external-components/external-panel/external-panel.ts",
  ],
  outdir: "dist/external",
  projectionManifest:
    "dist/external/webui-projection.json",
}));
