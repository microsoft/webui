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
  const metafileIndex = args.indexOf("--metafile");
  if (metafileIndex >= 0) {
    const rootsIndex = args.indexOf("--emit-component-assets");
    const roots = rootsIndex >= 0 ? args[rootsIndex + 1] : "";
    const outputs = roots === "root-z,root-a"
      ? {
          "a-shared-z.webui.js": {
            bytes: 1,
            inputs: { "webui:component/shared-z": { bytesInOutput: 1 } },
          },
          "a-root-z.webui.js": {
            bytes: 1,
            inputs: { "webui:component/root-z": { bytesInOutput: 1 } },
            entryPoint: "webui:component/root-z",
          },
          "z-shared-a.webui.js": {
            bytes: 1,
            inputs: { "webui:component/shared-a": { bytesInOutput: 1 } },
          },
          "z-root-a.webui.js": {
            bytes: 1,
            inputs: { "webui:component/root-a": { bytesInOutput: 1 } },
            entryPoint: "webui:component/root-a",
          },
        }
      : {
          "current-root.webui.js": {
            bytes: 1,
            inputs: { "webui:component/current-root": { bytesInOutput: 1 } },
            entryPoint: "webui:component/current-root",
          },
        };
    for (const assetName of Object.keys(outputs)) {
      fs.writeFileSync(
        path.join(outDir, assetName),
        'const asset={"type":"webui-component-asset"}; export default asset;'
      );
    }
    const metafilePath = args[metafileIndex + 1];
    fs.mkdirSync(path.dirname(metafilePath), { recursive: true });
    fs.writeFileSync(
      metafilePath,
      JSON.stringify({ inputs: {}, outputs })
    );
  }
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

  test("returns only assets from the current fallback build graph", (t) => {
    const root = fixtureRoot();
    t.after(() => rmSync(root, { recursive: true, force: true }));
    const appDir = path.join(root, "app");
    const outDir = path.join(root, "dist");
    mkdirSync(outDir);
    writeFileSync(
      path.join(outDir, "stale-root.webui.js"),
      'const asset={"type":"webui-component-asset"}; export default asset;'
    );
    const resultPath = path.join(root, "result.json");
    const result = runFallback(
      root,
      `
import { writeFileSync } from "node:fs";
import { build } from ${JSON.stringify(PACKAGE_ENTRY)};
const result = build({
  appDir: ${JSON.stringify(appDir)},
  outDir: ${JSON.stringify(outDir)},
  plugin: "webui",
  componentAssetRoots: ["current-root"],
});
writeFileSync(
  ${JSON.stringify(resultPath)},
  JSON.stringify(result.componentAssetFiles),
);
`
    );

    assert.equal(result.status, 0, result.stderr);
    const files = JSON.parse(readFileSync(resultPath, "utf8")) as string[];
    assert.deepEqual(files.slice(0, 1), ["current-root.webui.js"]);
    assert.equal(files.length, 2);
    assert.equal(files.includes("stale-root.webui.js"), false);
    const args = JSON.parse(
      readFileSync(path.join(root, "args.json"), "utf8")
    ) as string[];
    assert.notEqual(args.indexOf("--metafile"), -1);
  });

  test("rejects a metafile request without component asset roots", (t) => {
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
    componentAssetMetafile: true,
  });
  process.exit(2);
} catch (error) {
  if (!(error instanceof Error) || !error.message.includes("requires at least one")) {
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

  test("forwards even an empty asset filename template to native validation", (t) => {
    const root = fixtureRoot();
    t.after(() => rmSync(root, { recursive: true, force: true }));
    const appDir = path.join(root, "app");
    const outDir = path.join(root, "dist");
    const result = runFallback(
      root,
      `
import { build } from ${JSON.stringify(PACKAGE_ENTRY)};
build({
  appDir: ${JSON.stringify(appDir)},
  outDir: ${JSON.stringify(outDir)},
  cssFileNameTemplate: "",
});
`
    );

    assert.equal(result.status, 0, result.stderr);
    const args = JSON.parse(
      readFileSync(path.join(root, "args.json"), "utf8")
    ) as string[];
    const templateIndex = args.indexOf("--asset-file-name-template");
    assert.notEqual(templateIndex, -1);
    assert.equal(args[templateIndex + 1], "");
    assert.equal(args.includes("--css-file-name-template"), false);
  });

  test("returns roots and chunks in native graph order", (t) => {
    const root = fixtureRoot();
    t.after(() => rmSync(root, { recursive: true, force: true }));
    const appDir = path.join(root, "app");
    const outDir = path.join(root, "dist");
    const resultPath = path.join(root, "result.json");
    const result = runFallback(
      root,
      `
import { writeFileSync } from "node:fs";
import { build } from ${JSON.stringify(PACKAGE_ENTRY)};
const result = build({
  appDir: ${JSON.stringify(appDir)},
  outDir: ${JSON.stringify(outDir)},
  componentAssetRoots: ["root-z", "root-a"],
});
writeFileSync(
  ${JSON.stringify(resultPath)},
  JSON.stringify(result.componentAssetFiles),
);
`
    );

    assert.equal(result.status, 0, result.stderr);
    const files = JSON.parse(readFileSync(resultPath, "utf8")) as string[];
    assert.deepEqual(
      [files[0], files[2], files[4], files[6]],
      [
        "z-root-a.webui.js",
        "a-root-z.webui.js",
        "z-shared-a.webui.js",
        "a-shared-z.webui.js",
      ]
    );
  });

  test("uses a temporary metafile when returning fallback metadata", (t) => {
    const root = fixtureRoot();
    t.after(() => rmSync(root, { recursive: true, force: true }));
    const appDir = path.join(root, "app");
    const outDir = path.join(root, "dist");
    const resultPath = path.join(root, "result.json");
    const result = runFallback(
      root,
      `
import { writeFileSync } from "node:fs";
import { build } from ${JSON.stringify(PACKAGE_ENTRY)};
const result = build({
  appDir: ${JSON.stringify(appDir)},
  outDir: ${JSON.stringify(outDir)},
  componentAssetRoots: ["current-root"],
  componentAssetMetafile: true,
});
writeFileSync(
  ${JSON.stringify(resultPath)},
  JSON.stringify(result.componentAssetMetafile),
);
`
    );

    assert.equal(result.status, 0, result.stderr);
    assert.match(
      JSON.parse(readFileSync(resultPath, "utf8")) as string,
      /"outputs"/
    );
    const args = JSON.parse(
      readFileSync(path.join(root, "args.json"), "utf8")
    ) as string[];
    const metafilePath = args[args.indexOf("--metafile") + 1];
    assert.equal(metafilePath.startsWith(`${outDir}${path.sep}`), false);
  });
});
