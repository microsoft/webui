// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Validate the real platform tarballs, including bytes, identity, and license.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { tarInvocation } from "./node-package-archive.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const archives = process.argv.slice(2);
assert.ok(archives.length, "Pass one or more packed npm platform tarballs");

function tarText(archive, flags, ...entries) {
  const { args, cwd } = tarInvocation(archive, flags, ...entries);
  const result = spawnSync("tar", args, {
    cwd,
    encoding: "utf8",
    maxBuffer: 1024 * 1024,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}

async function digest(stream) {
  const hash = createHash("sha256");
  for await (const chunk of stream) hash.update(chunk);
  return hash.digest("hex");
}

async function entryDigest(archive, entry) {
  const { args, cwd } = tarInvocation(archive, "-xOf", entry);
  const child = spawn("tar", args, {
    cwd,
    stdio: ["ignore", "pipe", "inherit"],
  });
  const completed = new Promise((resolve) => {
    child.once("error", resolve);
    child.once("close", resolve);
  });
  const hash = await digest(child.stdout);
  assert.equal(await completed, 0, `Cannot extract ${archive}:${entry}`);
  return hash;
}

for (const archive of archives) {
  const inventory = tarText(archive, "-tzf").trim().split(/\r?\n/);
  const packed = JSON.parse(tarText(archive, "-xOf", "package/package.json"));
  assert.match(packed.name, /^@microsoft\/webui-(darwin|linux|win32)-(arm64|x64)$/);
  assert.equal(
    path.basename(archive),
    `${packed.name.slice(1).replace("/", "-")}-${packed.version}.tgz`,
    `${archive}: package identity does not match the tarball filename`,
  );
  const packageDir = path.join(root, "packages", packed.name.slice("@microsoft/".length));
  const source = JSON.parse(readFileSync(path.join(packageDir, "package.json"), "utf8"));
  for (const field of ["name", "version", "license", "cpu", "os", "files"]) {
    assert.deepEqual(packed[field], source[field], `${packed.name}: ${field}`);
  }
  assert.equal(source.license, "MIT");
  const cli = source.os[0] === "win32" ? "bin/webui.exe" : "bin/webui";
  for (const file of [cli, "webui.node", "webui-build.node", "LICENSE"]) {
    const entry = `package/${file}`;
    assert.equal(inventory.filter((name) => name === entry).length, 1, `${archive}: ${entry}`);
    const original = file === "LICENSE" ? path.join(root, file) : path.join(packageDir, file);
    assert.equal(
      await entryDigest(archive, entry),
      await digest(createReadStream(original)),
      `${archive}: incorrect ${file} bytes`,
    );
  }
  console.log(`Validated ${packed.name}@${packed.version}: CLI, handler, compiler, MIT license`);
}
