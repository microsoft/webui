// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Run in a fresh installed consumer, not through workspace source resolution.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";

const fixture = mkdtempSync(path.join(tmpdir(), "webui-packed-node-"));
const missing = path.join(fixture, "missing.node");

function run(source, env = {}) {
  const result = spawnSync(process.execPath, ["--input-type=module", "--eval", source], {
    cwd: process.cwd(),
    encoding: "utf8",
    timeout: 60_000,
    env: {
      ...process.env,
      WEBUI_ADDON_PATH: "",
      WEBUI_BUILD_ADDON_PATH: "",
      ...env,
    },
  });
  assert.ifError(result.error);
  return result;
}

const trace = `
  import assert from "node:assert/strict";
  import path from "node:path";
  const loaded = [];
  const dlopen = process.dlopen;
  process.dlopen = function(module, filename, ...args) {
    loaded.push(path.basename(filename));
    return dlopen.call(this, module, filename, ...args);
  };
`;

try {
  writeFileSync(path.join(fixture, "index.html"), "<p>Hello {{name}}</p>");
  const compiled = run(`${trace}
    const { build } = await import("@microsoft/webui/build");
    assert.deepEqual(loaded, []);
    const result = build({ appDir: ${JSON.stringify(fixture)} });
    assert.deepEqual(loaded, ["webui-build.node"]);
    console.log(result.protocol.toString("base64"));
  `, { WEBUI_ADDON_PATH: missing });
  assert.equal(compiled.status, 0, compiled.stderr);
  const protocol = compiled.stdout.trim();
  assert.ok(protocol.length > 0);

  const rendered = run(`${trace}
    const runtime = await import("@microsoft/webui");
    assert.deepEqual(loaded, []);
    assert.equal("build" in runtime, false);
    const protocol = new runtime.Protocol(Buffer.from(${JSON.stringify(protocol)}, "base64"));
    assert.equal(protocol.render({ name: "packaged" }).toString(), "<p>Hello packaged</p>");
    assert.deepEqual(loaded, ["webui.node"]);
  `, { WEBUI_BUILD_ADDON_PATH: missing });
  assert.equal(rendered.status, 0, rendered.stderr);

  const together = run(`${trace}
    const { build } = await import("@microsoft/webui/build");
    const { Protocol } = await import("@microsoft/webui");
    const result = build({ appDir: ${JSON.stringify(fixture)} });
    assert.ok(Buffer.isBuffer(result.protocol));
    const html = new Protocol(result.protocol).render({ name: "packaged" });
    assert.ok(Buffer.isBuffer(html));
    assert.equal(html.toString(), "<p>Hello packaged</p>");
    assert.deepEqual(loaded, ["webui-build.node", "webui.node"]);
  `);
  assert.equal(together.status, 0, together.stderr);

  const require = createRequire(path.join(process.cwd(), "package.json"));
  const platformPackage = `@microsoft/webui-${process.platform}-${process.arch}`;
  const incompatibleBuild = run(`
    const { build } = await import("@microsoft/webui/build");
    build({ appDir: ${JSON.stringify(fixture)} });
  `, { WEBUI_BUILD_ADDON_PATH: require.resolve(`${platformPackage}/webui.node`) });
  assert.notEqual(incompatibleBuild.status, 0);
  assert.match(incompatibleBuild.stderr, /Native build addon is incompatible: build\(\) is required/);

  const incompatibleRuntime = run(`
    const { Protocol } = await import("@microsoft/webui");
    new Protocol(Buffer.from(${JSON.stringify(protocol)}, "base64"));
  `, { WEBUI_ADDON_PATH: require.resolve(`${platformPackage}/webui-build.node`) });
  assert.notEqual(incompatibleRuntime.status, 0);
  assert.match(incompatibleRuntime.stderr, /Native addon is incompatible: Protocol is required/);

  // Missing/corrupt compiler artifacts must fail, never use the CLI or runtime.
  for (const broken of ["missing.node", "broken.node"]) {
    const file = path.join(fixture, broken);
    if (broken === "broken.node") writeFileSync(file, "not a native addon");
    const failed = run(`
      const { build } = await import("@microsoft/webui/build");
      build({ appDir: ${JSON.stringify(fixture)} });
    `, { WEBUI_BUILD_ADDON_PATH: file });
    assert.notEqual(failed.status, 0);
    assert.match(failed.stderr, /Failed to load native build addon/);
    assert.ok(failed.stderr.includes(file), failed.stderr);
  }
  console.log("Packed Node imports load only their selected native addon; compiler failures propagate");
} finally {
  rmSync(fixture, { recursive: true, force: true });
}
