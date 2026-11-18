// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from "node:fs/promises";
import path from "node:path";
import { test, type TestContext } from "node:test";
import { bundleClient, build, prepareBuild, inspect, Protocol } from "@microsoft/webui";

const project = path.resolve(process.cwd(), "../..");

async function fixture(t: TestContext) {
  const root = await mkdtemp(path.join(process.cwd(), ".tmp-native-bundle-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(path.join(root, "node_modules/@microsoft"), { recursive: true });
  await symlink(path.join(project, "packages/webui-framework"),
    path.join(root, "node_modules/@microsoft/webui-framework"),
    process.platform === "win32" ? "junction" : "dir");
  await writeFile(path.join(root, "tsconfig.json"), JSON.stringify({
    compilerOptions: { experimentalDecorators: true, useDefineForClassFields: false },
  }));
  await writeFile(path.join(root, "index.html"), "<app-counter></app-counter>");
  await writeFile(path.join(root, "app-counter.html"), "<p>{{count}}</p>");
  await writeFile(path.join(root, "app-counter.ts"), `
    import { WebUIElement, observable } from '@microsoft/webui-framework';
    export class Counter extends WebUIElement { @observable count = 0; }
    Counter.define('app-counter');
  `);
  await writeFile(path.join(root, "index.ts"), "import './app-counter.ts';");
  return {
    root,
    options: {
      project,
      entries: { index: path.join(root, "index.ts") },
      outDir: path.join(root, "dist"),
      tsconfig: path.join(root, "tsconfig.json"),
    },
  };
}

test("native bundling proves final bytes and finishes a retained renderable compilation", async t => {
  const { root, options } = await fixture(t);
  const prepared = prepareBuild({ appDir: root, plugin: "webui", modulesOut: path.join(root, ".webui") });
  prepared.writeModules();
  const client = await bundleClient(options);
  const component = client.manifest.components["app-counter"];
  assert.ok(component);
  assert.deepEqual(component.hydrationKeys, ["count"]);
  const javascript = await readFile(path.join(root, "dist/index.js"));
  assert.ok(javascript.every(byte => byte < 128));
  assert.ok(Object.values(client.manifest.outputs).includes(
    `sha256:${createHash("sha256").update(javascript).digest("hex")}`,
  ));
  const result = prepared.finish({
    projectionManifestObjects: [{ path: client.manifestPath, manifest: client.manifest }],
  });
  const protocol = new Protocol(result.protocol, { plugin: "webui" });
  assert.match(protocol.render({ count: 7 }).toString(), /7/);
  assert.deepEqual(
    JSON.parse(inspect(result.protocol)),
    JSON.parse(inspect(build({ appDir: root, plugin: "webui", projectionManifests: [client.manifestPath] }).protocol)),
  );
  await writeFile(path.join(root, "dist/index.js"), "stale output");
  assert.throws(() => build({
    appDir: root, plugin: "webui", projectionManifests: [client.manifestPath],
  }), /PROJ-M004/);
});

test("native in-memory builds retain split outputs, legal notices, ASCII and fresh graph evidence", async t => {
  const { root, options } = await fixture(t);
  await writeFile(path.join(root, "index.ts"),
    "/*! @license Fixture MIT */ export const value = '雪'; export const load = () => import('./feature.ts'); if (__WEBUI_DEV__) import('./diagnostic.ts');");
  await writeFile(path.join(root, "feature.ts"), "export const feature = 'Feature';");
  await writeFile(path.join(root, "diagnostic.ts"), "console.error('PRIVATE_NATIVE_DIAGNOSTIC');");
  const client = await bundleClient({ ...options, write: false, sourcemap: "file" });
  const outputs = client.files.filter(file => file.path.endsWith(".js"));
  assert.ok(outputs.length >= 2);
  for (const file of outputs) {
    assert.ok(file.contents.every(byte => byte < 128));
    assert.doesNotMatch(file.contents.toString(), /PRIVATE_NATIVE_DIAGNOSTIC/);
  }
  assert.ok(outputs.some(file => file.contents.toString().includes("@license Fixture MIT")));
  assert.ok(client.files.some(file => file.path.endsWith(".map")));
  await assert.rejects(readFile(path.join(root, "dist/index.js")), { code: "ENOENT" });
});

test("native path boundaries accept ordinary and namespaced absolute destinations without weakening containment", async t => {
  const { root, options } = await fixture(t);
  const manifestPath = path.join(root, "dist/proof.json");
  const variants = [
    { ...options, manifestPath },
    { ...options, outDir: path.relative(project, options.outDir),
      manifestPath: path.relative(project, manifestPath) },
    { ...options, project: path.toNamespacedPath(project), manifestPath },
    { ...options, outDir: path.toNamespacedPath(options.outDir),
      manifestPath: path.toNamespacedPath(manifestPath) },
  ];
  const source = await readFile(path.join(root, "index.ts"));
  const sourceKey = path.relative(project, path.join(root, "index.ts")).replaceAll("\\", "/");
  for (const variant of variants) {
    const client = await bundleClient({ ...variant, write: false });
    assert.equal(path.toNamespacedPath(client.manifestPath), path.toNamespacedPath(manifestPath));
    const output = client.files.find(file => path.basename(file.path) === "index.js");
    assert.ok(output);
    assert.equal(path.toNamespacedPath(output.path),
      path.toNamespacedPath(path.join(root, "dist/index.js")));
    assert.equal(client.manifest.inputs[sourceKey],
      `sha256:${createHash("sha256").update(source).digest("hex")}`);
    assert.deepEqual(client.manifest.components["app-counter"].hydrationKeys, ["count"]);
  }
  await assert.rejects(readFile(manifestPath), { code: "ENOENT" });
  for (const outDir of [
    path.join(`${project}-outside`, "dist"),
    path.join(project, "..") + `${path.sep}escape`,
    `dist${path.sep}..${path.sep}escape`,
  ]) {
    await assert.rejects(bundleClient({ ...options, outDir, write: false }), /inside|parent/);
  }
  await assert.rejects(bundleClient({
    ...options, manifestPath: path.join(`${project}-outside`, "proof.json"), write: false,
  }), /inside/);
  const published = await bundleClient({ ...options, manifestPath });
  assert.deepEqual(JSON.parse(await readFile(manifestPath, "utf8")), published.manifest);
  assert.ok((await readFile(path.join(root, "dist/index.js"))).length > 0);
});

test("native publication rejects source/manifest collisions and redirected output paths", async t => {
  const { root, options } = await fixture(t);
  await assert.rejects(bundleClient({ ...options, manifestPath: path.join(root, "dist/index.js") }),
    /manifest cannot replace/);
  if (process.platform !== "win32") {
    const outside = await mkdtemp(path.join(process.cwd(), ".tmp-native-external-"));
    t.after(() => rm(outside, { recursive: true, force: true }));
    await mkdir(path.join(root, "redirect"));
    await symlink(outside, path.join(root, "redirect/public"));
    await assert.rejects(bundleClient({ ...options, outDir: path.join(root, "redirect/public") }),
      /redirected/);
    await assert.rejects(readFile(path.join(outside, "index.js")), { code: "ENOENT" });
  }
});

test("native bundling returns typed nonfatal advisories without terminal colors", async t => {
  const { root, options } = await fixture(t);
  await writeFile(path.join(root, "index.ts"),
    "export { feature } from './feature.ts'; export const load = () => import('./feature.ts');");
  await writeFile(path.join(root, "feature.ts"), "export const feature = 'shared';");
  const client = await bundleClient({ ...options, write: false });
  assert.equal(client.warnings.length, 1);
  assert.equal(client.warnings[0].code, "INEFFECTIVE_DYNAMIC_IMPORT");
  assert.ok(client.warnings[0].message.length > 0);
  assert.doesNotMatch(client.warnings[0].message, /\x1b/);
  assert.ok(client.files.some(file => path.basename(file.path) === "index.js"));
});
