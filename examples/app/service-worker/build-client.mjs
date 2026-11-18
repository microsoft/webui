// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { runWebUIClientBuild } from "../../build-client.mjs";
import { fileURLToPath } from "node:url";

await runWebUIClientBuild({
  entries: {
    app: fileURLToPath(new URL("src/app.ts", import.meta.url)),
    "service-worker": fileURLToPath(new URL("src/service-worker.ts", import.meta.url)),
  },
  outDir: "public",
  target: "es2022",
  external: ["./wasm/handler/webui_wasm_handler.js"],
});
