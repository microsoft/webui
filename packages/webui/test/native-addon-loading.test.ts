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
import { build } from "@microsoft/webui/build";
import { resolve as resolveNative } from "@microsoft/webui/platform.js";

const PACKAGE_ENTRY = pathToFileURL(
  path.resolve("dist", "index.js")
).href;
const BUILD_ENTRY = pathToFileURL(path.resolve("dist", "build.js")).href;

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
        import { build } from ${JSON.stringify(BUILD_ENTRY)};
        build({ appDir: "." });
      `,
    ],
    {
      cwd: root,
      encoding: "utf8",
      env: {
        ...process.env,
        WEBUI_BUILD_ADDON_PATH: addonPath,
        WEBUI_BINARY_PATH: process.execPath,
        WEBUI_CLI_MARKER: cliMarker,
      },
    },
  );

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /\[webui\] Failed to load native build addon at /);
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
        const { build } = await import(${JSON.stringify(BUILD_ENTRY)});
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
        WEBUI_BUILD_ADDON_PATH: addonPath,
      },
    },
  );

  assert.equal(result.status, 0, result.stderr);
  const error = JSON.parse(result.stdout) as {
    message: string;
    cause: string;
  };
  assert.match(error.message, /\[webui\] Failed to load native build addon at /);
  assert.match(error.message, /Unsupported platform: unsupported-os-/);
  assert.notEqual(error.cause, "");
});

test("runtime and compiler imports load only their own native binaries", t => {
  const root = mkdtempSync(path.join(tmpdir(), "webui-addon-boundary-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  writeFileSync(path.join(root, "index.html"), "<h1>Hello {{name}}</h1>");
  writeFileSync(path.join(root, "index.ts"), "export const message = 'browser';");
  const compiled = build({ appDir: root });
  const protocolPath = path.join(root, "protocol.bin");
  writeFileSync(protocolPath, compiled.protocol);
  const runtime = resolveNative("addon");
  const compiler = resolveNative("build-addon");
  assert.ok(runtime);
  assert.ok(compiler);
  assert.notEqual(runtime, compiler);
  const broken = path.join(root, "unrelated-broken.node");
  writeFileSync(broken, "the unselected addon must not load");
  const capture = `
    import assert from 'node:assert/strict';
    const loaded = [];
    const dlopen = process.dlopen;
    process.dlopen = function(module, filename, flags) {
      loaded.push(filename);
      return dlopen.call(this, module, filename, flags);
    };
    const runtime = await import(${JSON.stringify(PACKAGE_ENTRY)});
    const compiler = await import(${JSON.stringify(BUILD_ENTRY)});
    assert.deepEqual(loaded, []);
    assert.equal('build' in runtime, false);
    assert.equal('bundleClient' in runtime, false);
    assert.equal('Protocol' in compiler, false);
  `;
  const scenarios = [
    {
      env: { WEBUI_ADDON_PATH: runtime, WEBUI_BUILD_ADDON_PATH: broken },
      script: `
        const { readFileSync } = await import('node:fs');
        const protocol = new runtime.Protocol(readFileSync(${JSON.stringify(protocolPath)}));
        assert.match(protocol.render({name:'runtime'}).toString(), /Hello runtime/);
        const state = protocol.prepareState({name:'snapshot'});
        assert.match(protocol.renderPrepared(state).toString(), /Hello snapshot/);
        assert.ok(JSON.parse(runtime.inspect(readFileSync(${JSON.stringify(protocolPath)}))));
        assert.deepEqual(loaded, [${JSON.stringify(runtime)}]);
      `,
    },
    {
      env: { WEBUI_ADDON_PATH: broken, WEBUI_BUILD_ADDON_PATH: compiler },
      script: `
        const result = compiler.build({appDir:${JSON.stringify(root)}});
        assert.ok(result.protocol.length > 0);
        const prepared = compiler.prepareBuild({appDir:${JSON.stringify(root)}});
        assert.deepEqual(prepared.finish().protocol, result.protocol);
        const browser = await compiler.bundleClient({
          project:${JSON.stringify(root)}, entries:{index:'index.ts'}, outDir:'dist',write:false,
        });
        assert.ok(browser.files.length > 0);
        assert.deepEqual(loaded, [${JSON.stringify(compiler)}]);
      `,
    },
  ];
  for (const { env, script } of scenarios) {
    const result = spawnSync(process.execPath, ["--input-type=module", "--eval", capture + script], {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env, ...env },
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stderr, "");
  }
});
