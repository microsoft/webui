// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";

const PACKAGE_ENTRY = pathToFileURL(
  path.resolve("dist", "index.js")
).href;

test("build propagates native addon load failures without invoking the CLI", (t) => {
  const root = mkdtempSync(path.join(tmpdir(), "webui-native-load-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));

  const addonPath = path.join(root, "broken.node");
  const cliMarker = path.join(root, "cli-invoked");
  writeFileSync(addonPath, "not a native addon");
  writeFileSync(
    path.join(root, "build"),
    `require("node:fs").writeFileSync(process.env.WEBUI_CLI_MARKER, "");`,
  );

  const result = spawnSync(
    process.execPath,
    [
      "--input-type=module",
      "--eval",
      `
        import { build } from ${JSON.stringify(PACKAGE_ENTRY)};
        build({ appDir: "." });
      `,
    ],
    {
      cwd: root,
      encoding: "utf8",
      env: {
        ...process.env,
        WEBUI_ADDON_PATH: addonPath,
        WEBUI_BINARY_PATH: process.execPath,
        WEBUI_CLI_MARKER: cliMarker,
      },
    },
  );

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /\[webui\] Failed to load native addon at /);
  assert.match(result.stderr, /broken\.node/);
  assert.equal(existsSync(cliMarker), false);
});

test("unsupported platform guidance preserves the addon load cause", (t) => {
  const root = mkdtempSync(path.join(tmpdir(), "webui-native-platform-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));

  const addonPath = path.join(root, "broken.node");
  writeFileSync(addonPath, "not a native addon");

  const result = spawnSync(
    process.execPath,
    [
      "--input-type=module",
      "--eval",
      `
        Object.defineProperty(process, "platform", { value: "unsupported-os" });
        const { build } = await import(${JSON.stringify(PACKAGE_ENTRY)});
        try {
          build({ appDir: "." });
          process.exit(2);
        } catch (error) {
          console.log(JSON.stringify({
            message: error instanceof Error ? error.message : String(error),
            cause: error instanceof Error && error.cause instanceof Error
              ? error.cause.message
              : "",
          }));
        }
      `,
    ],
    {
      cwd: root,
      encoding: "utf8",
      env: {
        ...process.env,
        WEBUI_ADDON_PATH: addonPath,
      },
    },
  );

  assert.equal(result.status, 0, result.stderr);
  const error = JSON.parse(result.stdout) as {
    message: string;
    cause: string;
  };
  assert.match(error.message, /\[webui\] Failed to load native addon at /);
  assert.match(error.message, /Unsupported platform: unsupported-os-/);
  assert.notEqual(error.cause, "");
});
