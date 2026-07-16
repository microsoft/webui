// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { describe, test } from "node:test";

const PACKAGE_ENTRY = pathToFileURL(
  path.resolve("dist", "index.js")
).href;

function fixtureRoot(): string {
  const root = mkdtempSync(path.join(tmpdir(), "webui-cli-fallback-"));
  writeFileSync(
    path.join(root, "build"),
    `
const fs = require("node:fs");
const path = require("node:path");
const args = process.argv.slice(2);
fs.writeFileSync(process.env.WEBUI_CAPTURE_PATH, JSON.stringify(args));
const outIndex = args.indexOf("--out");
if (outIndex >= 0) {
  const outDir = args[outIndex + 1];
  fs.mkdirSync(outDir, { recursive: true });
  fs.writeFileSync(path.join(outDir, "protocol.bin"), "protocol");
}
`
  );
  mkdirSync(path.join(root, "app"));
  return root;
}

function runFallback(root: string, source: string) {
  return spawnSync(
    process.execPath,
    ["--input-type=module", "--eval", source],
    {
      cwd: root,
      encoding: "utf8",
      env: {
        ...process.env,
        WEBUI_ADDON_PATH: path.join(root, "missing-addon.node"),
        WEBUI_BINARY_PATH: process.execPath,
        WEBUI_CAPTURE_PATH: path.join(root, "args.json"),
      },
    }
  );
}

describe("projection CLI fallback", () => {
  test("forwards manifest paths without requiring components", (t) => {
    const root = fixtureRoot();
    t.after(() => rmSync(root, { recursive: true, force: true }));
    const appDir = path.join(root, "app");
    const outDir = path.join(root, "dist");
    const manifest = path.join(root, "webui-projection.json");
    const result = runFallback(
      root,
      `
import { build } from ${JSON.stringify(PACKAGE_ENTRY)};
build({
  appDir: ${JSON.stringify(appDir)},
  outDir: ${JSON.stringify(outDir)},
  plugin: "webui",
  projectionManifests: [${JSON.stringify(manifest)}],
});
`
    );

    assert.equal(result.status, 0, result.stderr);
    const args = JSON.parse(
      readFileSync(path.join(root, "args.json"), "utf8")
    ) as string[];
    const manifestIndex = args.indexOf("--projection-manifest");
    assert.notEqual(manifestIndex, -1);
    assert.equal(args[manifestIndex + 1], manifest);
    assert.equal(args.includes("--components"), false);
  });

  test("rejects inline manifests before invoking the CLI", (t) => {
    const root = fixtureRoot();
    t.after(() => rmSync(root, { recursive: true, force: true }));
    const appDir = path.join(root, "app");
    const outDir = path.join(root, "dist");
    const result = runFallback(
      root,
      `
import { build } from ${JSON.stringify(PACKAGE_ENTRY)};
try {
  build({
    appDir: ${JSON.stringify(appDir)},
    outDir: ${JSON.stringify(outDir)},
    projectionManifestObjects: [{
      path: ${JSON.stringify(path.join(root, "webui-projection.json"))},
      manifest: {},
    }],
  });
  process.exit(2);
} catch (error) {
  if (!(error instanceof Error) || !error.message.includes("require the native addon")) {
    throw error;
  }
}
`
    );

    assert.equal(result.status, 0, result.stderr);
    assert.throws(
      () => readFileSync(path.join(root, "args.json"), "utf8"),
      /ENOENT/
    );
  });
});
