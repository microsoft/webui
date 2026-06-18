// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { spawnSync } from "node:child_process";
import { strict as assert } from "node:assert";
import { describe, test } from "node:test";
import { fileURLToPath } from "node:url";

describe("CLI launcher", () => {
  test("dispatches to WEBUI_BINARY_PATH", () => {
    const launcher = fileURLToPath(new URL("../../bin/webui", import.meta.url));
    const result = spawnSync(process.execPath, [launcher, "--version"], {
      encoding: "utf8",
      env: {
        ...process.env,
        WEBUI_BINARY_PATH: process.execPath,
      },
    });

    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout.trim(), /^v\d+\./);
  });
});
