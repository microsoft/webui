// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readFile, rm, stat, utimes, writeFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { promisify } from "node:util";

const exec = promisify(execFile);

test("parallel postbuild installs leave identical shared executables untouched and replace stale copies", async t => {
  const root = await mkdtemp(path.join(process.cwd(), ".tmp-native-install-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(path.join(root, "dist"));
  await writeFile(path.join(root, "package.json"), '{"type":"module"}');
  for (const file of ["install.js", "platform.js"]) {
    await copyFile(path.join(process.cwd(), "dist", file), path.join(root, "dist", file));
  }
  const source = path.join(root, "source-cli");
  const initial = "native-cli-v1".repeat(12000);
  const updated = `${initial.slice(0, -2)}v2`;
  await writeFile(source, initial);
  const install = () => exec(process.execPath, [path.join(root, "dist/install.js")], {
    env: { ...process.env, WEBUI_BINARY_PATH: source },
  });
  await install();
  const destinations = ["webui", ...(process.platform === "win32" ? ["webui.exe"] : [])]
    .map(name => path.join(root, "bin", name));
  const old = new Date("2020-01-01T00:00:00Z");
  for (const destination of destinations) await utimes(destination, old, old);
  const results = await Promise.all(Array.from({ length: 4 }, install));
  for (const result of results) assert.equal(result.stderr, "");
  for (const destination of destinations) {
    assert.equal(await readFile(destination, "utf8"), initial);
    assert.equal((await stat(destination)).mtimeMs, old.getTime());
  }
  await writeFile(source, updated);
  assert.equal((await install()).stderr, "");
  for (const destination of destinations) {
    assert.equal(await readFile(destination, "utf8"), updated);
  }
});
