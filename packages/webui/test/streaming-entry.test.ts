// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import { randomUUID } from "node:crypto";
import { access, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import * as path from "node:path";
import { gzipSync } from "node:zlib";
import { test, type TestContext } from "node:test";
import * as esbuild from "esbuild";
import { esbuildProjection } from "@microsoft/webui/projection.js";

const SOURCE = path.resolve("..", "webui-framework", "src");
const RUNTIME = "@microsoft/webui-framework/streaming.js";

async function fixture(t: TestContext): Promise<string> {
  const root = path.resolve(`.streaming-fixture-${randomUUID()}`);
  await mkdir(root);
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, "package.json"), '{"name":"streaming-test","type":"module","sideEffects":false}');
  await writeFile(path.join(root, "streaming.ts"), `import '${RUNTIME}';`);
  await writeFile(path.join(root, "app.ts"), `
import { WebUIElement } from '@microsoft/webui-framework';
class Card extends WebUIElement {}
Card.define('streaming-card');
export const applicationOnly = '${"unrelated-application".repeat(1024)}';
`);
  return root;
}

function options(root: string, extra: esbuild.BuildOptions = {}): esbuild.BuildOptions {
  return {
    absWorkingDir: root,
    entryPoints: { app: "app.ts", streaming: "streaming.ts" },
    outdir: "dist",
    bundle: true,
    format: "esm",
    splitting: true,
    platform: "browser",
    write: false,
    logLevel: "silent",
    metafile: true,
    alias: {
      "@microsoft/webui-framework": path.join(SOURCE, "index.ts"),
      [RUNTIME]: path.join(SOURCE, "streaming-entry.ts"),
    },
    ...extra,
  };
}

function streamingOutput(result: esbuild.BuildResult): string {
  const meta = result.metafile!;
  const entry = Object.keys(meta.outputs).find(id => meta.outputs[id]!.entryPoint === "streaming.ts");
  assert.ok(entry, "the authored entry must identify its own emitted output");
  return entry;
}

test("explicit streaming import preserves initialization, footprint and shared runtime identity", async t => {
  const root = await fixture(t);
  const result = await esbuild.build(options(root, {
    publicPath: "https://cdn.example.test/assets",
    entryNames: "[name]-[hash]",
    chunkNames: "chunks/[name]-[hash]",
    banner: { js: "/* Application license */" },
    minify: true,
    define: { __WEBUI_DEV__: "false" },
  }));
  const meta = result.metafile!;
  const early = new Set([streamingOutput(result)]);
  for (const id of early) {
    for (const edge of meta.outputs[id]!.imports) {
      if (edge.kind === "import-statement") {
        assert.equal(edge.external, undefined);
        early.add(edge.path);
      }
    }
  }
  let bytes = 0;
  let compressed = 0;
  let initialized = false;
  for (const id of early) {
    const file = result.outputFiles!.find(file => file.path === path.resolve(root, id))!;
    bytes += file.contents.length;
    compressed += gzipSync(file.contents).length;
    for (const [input, contribution] of Object.entries(meta.outputs[id]!.inputs)) {
      if (!contribution.bytesInOutput || input === "streaming.ts") continue;
      assert.ok(path.resolve(root, input).startsWith(SOURCE + path.sep), input);
      const name = path.basename(input);
      initialized ||= name === "streaming-entry.ts";
      assert.equal(["template-element.ts", "styles.ts", "link-styles.ts"].includes(name), false);
    }
  }
  assert.ok(initialized, "the explicit side-effect import must survive tree shaking");
  assert.ok(bytes <= 32 * 1024, `${bytes} initial bytes exceed the 32 KiB budget`);
  assert.ok(compressed <= 11 * 1024, `${compressed} gzip bytes exceed the 11 KiB budget`);
  assert.equal(result.outputFiles!.some(file => file.path.endsWith(".json")), false);
  for (const module of ["lifecycle.ts", "template-registry.ts", "style-catalog.ts"]) {
    const owners = Object.values(meta.outputs).filter(output =>
      Object.entries(output.inputs).some(([id, data]) => path.basename(id) === module && data.bytesInOutput > 0)
    );
    assert.equal(owners.length, 1, `${module} must have one module identity`);
  }
  await assert.rejects(access(path.join(root, "dist")));
});

test("omitting the explicit import never installs streaming implicitly", async t => {
  const root = await fixture(t);
  await writeFile(path.join(root, "streaming.ts"), "export {};");
  const result = await esbuild.build(options(root));
  assert.equal(Object.keys(result.metafile!.inputs).some(id => id.endsWith("/streaming-entry.ts")), false);
  assert.equal(Object.keys(result.metafile!.inputs).some(id => id.endsWith("/streaming-coordinator.ts")), false);
});

test("normal rebuilds retain explicit entry metadata without generated manifests", async t => {
  const root = await fixture(t);
  const context = await esbuild.context(options(root, { entryNames: "[name]-[hash]" }));
  try {
    const first = await context.rebuild();
    await writeFile(path.join(root, "app.ts"), "export const changedApplication = true;");
    const second = await context.rebuild();
    const third = await context.rebuild();
    assert.equal(streamingOutput(second), streamingOutput(third));
    assert.notDeepEqual(Object.keys(first.metafile!.outputs), Object.keys(second.metafile!.outputs));
    assert.equal(third.outputFiles!.some(file => file.path.endsWith(".json")), false);
  } finally {
    await context.dispose();
  }
});

test("explicit entries coexist with projection without streaming build tooling", async t => {
  const root = await fixture(t);
  const result = await esbuild.build(options(root, { write: true, plugins: [esbuildProjection()] }));
  assert.ok((await readFile(path.resolve(root, streamingOutput(result)))).length > 0);
  await access(path.join(root, "dist", "webui-projection.json"));
  await assert.rejects(access(path.join(root, "dist", "webui-streaming.json")));
});
