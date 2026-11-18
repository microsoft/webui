// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { tarInvocation } from "./node-package-archive.mjs";

function fixture(t) {
  const root = mkdtempSync(path.join(tmpdir(), "webui archive paths "));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
}

function checked(program, args, options) {
  const result = spawnSync(program, args, { ...options, encoding: "utf8" });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}

test("tar reads real archives with spaces and drive/filename colons using portable arguments", t => {
  const root = fixture(t);
  // Windows supplies an actual drive colon in its absolute temporary path.
  const name = process.platform === "win32" ? "archive with spaces.tgz" : "D: archive with spaces.tgz";
  writeFileSync(path.join(root, "payload.txt"), "compiler bytes");
  checked("tar", ["-czf", `./${name}`, "payload.txt"], { cwd: root });
  const archive = path.join(root, name);
  const list = tarInvocation(archive, "-tzf");
  assert.equal(list.cwd, path.dirname(archive));
  assert.deepEqual(list.args, ["-tzf", `./${name}`]);
  assert.match(checked("tar", list.args, { cwd: list.cwd }), /payload\.txt/);
  const extract = tarInvocation(archive, "-xOf", "payload.txt");
  assert.deepEqual(extract.args, ["-xOf", `./${name}`, "payload.txt"]);
  assert.equal(checked("tar", extract.args, { cwd: extract.cwd }), "compiler bytes");
});

test("platform inventory and streamed hashes validate an absolute archive in a colon directory", t => {
  const root = fixture(t);
  const scripts = path.join(root, "scripts");
  mkdirSync(scripts);
  for (const name of ["validate-node-packages.mjs", "node-package-archive.mjs"]) {
    copyFileSync(new URL(name, import.meta.url), path.join(scripts, name));
  }
  const manifest = {
    name: "@microsoft/webui-win32-x64", version: "3.1.2", license: "MIT",
    os: ["win32"], cpu: ["x64"], files: ["bin/webui.exe", "webui.node", "webui-build.node"],
  };
  const source = path.join(root, "packages/webui-win32-x64");
  const output = path.join(root, process.platform === "win32" ? "archives with spaces" : "D: archives with spaces");
  const packed = path.join(output, "package");
  for (const dir of [source, packed]) {
    mkdirSync(path.join(dir, "bin"), { recursive: true });
    writeFileSync(path.join(dir, "package.json"), JSON.stringify(manifest));
    for (const name of manifest.files) writeFileSync(path.join(dir, name), name);
  }
  writeFileSync(path.join(root, "LICENSE"), "MIT fixture");
  writeFileSync(path.join(packed, "LICENSE"), "MIT fixture");
  const name = "microsoft-webui-win32-x64-3.1.2.tgz";
  const archive = path.join(output, name);
  checked("tar", ["-czf", `./${name}`, "package"], { cwd: output });
  const script = path.join(scripts, "validate-node-packages.mjs");
  assert.match(checked(process.execPath, [script, archive], { cwd: root }), /CLI, handler, compiler, MIT license/);

  writeFileSync(path.join(packed, "webui-build.node"), "corrupt compiler bytes");
  checked("tar", ["-czf", `./${name}`, "package"], { cwd: output });
  const corrupt = spawnSync(process.execPath, [script, archive], { cwd: root, encoding: "utf8" });
  assert.ifError(corrupt.error);
  assert.notEqual(corrupt.status, 0);
  assert.match(corrupt.stderr, /incorrect webui-build\.node bytes/);
});
