// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { spawnSync } from "node:child_process";
import { strict as assert } from "node:assert";

const result = spawnSync(
  "cargo",
  [
    "run",
    "-p",
    "microsoft-webui-cli",
    "--",
    "build",
    "./src",
    "--plugin=webui",
    "--components",
    "../external-components",
    "--emit-component-assets",
    "lazy-panel,external-panel",
    "--projection-manifest",
    "./dist/webui-projection.json",
    "--out",
    "./dist/missing-fragment-check",
  ],
  {
    cwd: new URL(".", import.meta.url),
    encoding: "utf8",
    shell: process.platform === "win32",
  },
);

const output = `${result.stdout ?? ""}${result.stderr ?? ""}`;
assert.notEqual(result.status, 0, output);
assert.match(output, /PROJ-B001/);
