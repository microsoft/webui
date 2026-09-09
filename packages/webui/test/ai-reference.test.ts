// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import { execFileSync, spawnSync } from "node:child_process";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { test } from "node:test";
import type { TestContext } from "node:test";

async function createFixture(t: TestContext) {
  const root = await mkdtemp(path.join(tmpdir(), "webui-ai-reference-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const packageRoot = path.join(root, "packages", "webui");
  const script = path.join(packageRoot, "scripts", "prepare-ai.js");
  const source = path.join(root, "docs", "ai.md");
  await mkdir(path.dirname(script), { recursive: true });
  await mkdir(path.dirname(source), { recursive: true });
  await copyFile(path.resolve("scripts", "prepare-ai.js"), script);
  await copyFile(
    path.resolve("package.json"),
    path.join(packageRoot, "package.json"),
  );
  return { root, packageRoot, script, source };
}

const readReference = "require('node:fs').readFileSync(require.resolve('@microsoft/webui/ai.md'), 'utf8')";

test("dependency build hooks do not rewrite the package-only reference", async () => {
  const manifest = JSON.parse(await readFile(path.resolve("package.json"), "utf8"));
  for (const hook of ["prebuild", "build", "postbuild"]) {
    assert.doesNotMatch(
      manifest.scripts[hook] ?? "",
      /prepare-ai/,
      `${hook} must not race parallel dependency builds for ai.md`,
    );
  }
});

test("prepack refreshes the reference from the canonical document", async (t) => {
  const { root, packageRoot, script, source } = await createFixture(t);
  const manifest = JSON.parse(
    await readFile(path.join(packageRoot, "package.json"), "utf8"),
  );
  assert.ok(manifest.files.includes("ai.md"));
  assert.equal(manifest.scripts.prepack, "node scripts/prepare-ai.js");
  const canonical = await readFile(path.resolve("../../docs/ai.md"));
  await writeFile(source, canonical);
  execFileSync(process.execPath, [script], { cwd: root });
  assert.deepEqual(await readFile(path.join(packageRoot, "ai.md")), canonical);

  const updated = Buffer.concat([
    canonical,
    Buffer.from("\nUpdated release guidance.\n"),
  ]);
  await writeFile(source, updated);
  execFileSync(process.execPath, [script], { cwd: packageRoot });
  assert.deepEqual(await readFile(path.join(packageRoot, "ai.md")), updated);
});

test("the loader resolves hoisted and app-local versions without loading native code", async (t) => {
  const { root, packageRoot, script, source } = await createFixture(t);
  await writeFile(source, "Hoisted WebUI reference\n");
  execFileSync(process.execPath, [script]);
  const hoistedScope = path.join(root, "node_modules", "@microsoft");
  await mkdir(hoistedScope, { recursive: true });
  await symlink(packageRoot, path.join(hoistedScope, "webui"), "junction");
  const app = path.join(root, "apps", "my-app");
  await mkdir(app, { recursive: true });
  assert.equal(
    execFileSync(process.execPath, ["-p", readReference], {
      cwd: app,
      encoding: "utf8",
    }),
    "Hoisted WebUI reference\n\n",
  );

  const localPackage = path.join(app, "node_modules", "@microsoft", "webui");
  await mkdir(localPackage, { recursive: true });
  await copyFile(
    path.join(packageRoot, "package.json"),
    path.join(localPackage, "package.json"),
  );
  await writeFile(path.join(localPackage, "ai.md"), "App-local WebUI reference\n");
  assert.equal(
    execFileSync(process.execPath, ["-p", readReference], {
      cwd: app,
      encoding: "utf8",
    }),
    "App-local WebUI reference\n\n",
  );
});

test("preparing a reference fails when the canonical document is missing", async (t) => {
  const { root, packageRoot, script } = await createFixture(t);
  await writeFile(path.join(packageRoot, "ai.md"), "Stale guidance");
  const result = spawnSync(process.execPath, [script], {
    cwd: root,
    encoding: "utf8",
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /ENOENT/);
  assert.match(result.stderr, /ai\.md/);
});
