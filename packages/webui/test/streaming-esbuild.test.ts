// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import { randomUUID } from "node:crypto";
import { access, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import * as path from "node:path";
import { gzipSync } from "node:zlib";
import { test, type TestContext } from "node:test";
import * as esbuild from "esbuild";
import { esbuildStreaming, getStreamingAsset, StreamingBuildError } from "@microsoft/webui/streaming.js";
import { esbuildProjection } from "@microsoft/webui/projection.js";

const SOURCE = path.resolve("..", "webui-framework", "src");
const ENTRY = "webui-streaming:coordinator";
const RUNTIME = "@microsoft/webui-framework/streaming.js";

async function fixture(t: TestContext): Promise<string> {
  const root = path.resolve(`.streaming-fixture-${randomUUID()}`);
  await mkdir(root);
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, "package.json"), '{"name":"streaming-test","type":"module"}');
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
    entryPoints: { app: "app.ts" },
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
    plugins: [esbuildStreaming()],
    ...extra,
  };
}

function diagnostic(code: string): (error: unknown) => boolean {
  return error => {
    if (error instanceof StreamingBuildError) {
      assert.equal(error.code, code);
      assert.ok(error.message.includes("help:"));
      return true;
    }
    assert.ok(error instanceof Error && "errors" in error);
    const failures = (error as esbuild.BuildFailure).errors;
    assert.ok(failures.some(failure =>
      failure.id === code ||
      (failure.detail instanceof StreamingBuildError && failure.detail.code === code)
    ), failures.map(failure => failure.text).join("\n"));
    return true;
  };
}

test("identifies hashed assets and enforces the real early-runtime footprint", async t => {
  const root = await fixture(t);
  const result = await esbuild.build(options(root, {
    publicPath: "https://cdn.example.test/assets",
    entryNames: "[name]-[hash]",
    chunkNames: "chunks/[name]-[hash]",
    minify: true,
    define: { __WEBUI_DEV__: "false" },
  }));
  const asset = getStreamingAsset(result);
  const meta = result.metafile!;
  const entry = Object.keys(meta.outputs).find(id => meta.outputs[id]!.entryPoint === ENTRY)!;
  const early = new Set([entry]);
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
  for (const id of early) {
    const file = result.outputFiles!.find(file => file.path === path.resolve(root, id))!;
    bytes += file.contents.length;
    compressed += gzipSync(file.contents).length;
    for (const [input, contribution] of Object.entries(meta.outputs[id]!.inputs)) {
      if (!contribution.bytesInOutput || input === ENTRY) continue;
      assert.ok(path.resolve(root, input).startsWith(SOURCE + path.sep), input);
      assert.equal(["template-element.ts", "styles.ts", "link-styles.ts"].includes(path.basename(input)), false);
    }
  }
  assert.ok(bytes <= 32 * 1024, `${bytes} initial bytes exceed the 32 KiB budget`);
  assert.ok(compressed <= 11 * 1024, `${compressed} gzip bytes exceed the 11 KiB budget`);
  assert.equal(asset.entry, entry, "return the output key, not a guessed URL");
  const expected = [...early].filter(id => id !== entry).sort((a, b) =>
    meta.outputs[b]!.bytes - meta.outputs[a]!.bytes || Buffer.compare(Buffer.from(a), Buffer.from(b))
  );
  assert.deepEqual(asset.imports, expected);
  assert.equal(result.outputFiles!.some(file => file.path.endsWith(".json")), false);
  for (const module of ["lifecycle.ts", "template-registry.ts", "style-catalog.ts"]) {
    const owners = Object.values(meta.outputs).filter(output =>
      Object.entries(output.inputs).some(([id, data]) => path.basename(id) === module && data.bytesInOutput > 0)
    );
    assert.equal(owners.length, 1, `${module} must have one module identity`);
  }
  await assert.rejects(access(path.join(root, "dist")));
});

test("rebuilds deterministic descriptors without retaining obsolete application output", async t => {
  const root = await fixture(t);
  const context = await esbuild.context(options(root, { entryNames: "[name]-[hash]" }));
  try {
    const first = await context.rebuild();
    await writeFile(path.join(root, "app.ts"), "export const changedApplication = true;");
    const second = await context.rebuild();
    const third = await context.rebuild();
    assert.deepEqual(getStreamingAsset(second), getStreamingAsset(third));
    assert.notDeepEqual(Object.keys(first.metafile!.outputs), Object.keys(second.metafile!.outputs));
  } finally {
    await context.dispose();
  }
});

test("coexists with projection in either plugin order without another disk artifact", async t => {
  const root = await fixture(t);
  for (const plugins of [
    [esbuildStreaming(), esbuildProjection()],
    [esbuildProjection(), esbuildStreaming()],
  ]) {
    const result = await esbuild.build(options(root, { write: true, plugins }));
    const asset = getStreamingAsset(result);
    assert.ok((await readFile(path.resolve(root, asset.entry))).length > 0);
    await access(path.join(root, "dist", "webui-projection.json"));
    await assert.rejects(access(path.join(root, "dist", "webui-streaming.json")));
  }
  await writeFile(path.join(root, "app.ts"), "export const broken = ;");
  await assert.rejects(esbuild.build(options(root)));
});

test("rejects configurations that introduce global code or duplicate module identities", async t => {
  const root = await fixture(t);
  for (const extra of [
    { format: "iife" as const },
    { splitting: false },
    { inject: [ENTRY] },
    { preserveSymlinks: true },
    { plugins: [esbuildStreaming(), esbuildStreaming()] },
  ]) {
    await assert.rejects(esbuild.build(options(root, extra)), diagnostic("STREAM-B001"));
  }
});

test("rejects external runtimes, wrong framework aliases and contaminated early chunks", async t => {
  const root = await fixture(t);
  await assert.rejects(esbuild.build(options(root, {
    external: [path.join(SOURCE, "streaming-entry.ts")],
  })), diagnostic("STREAM-B002"));
  const aliased = await esbuild.build(options(root, {
    alias: {
      "@microsoft/webui-framework": path.join(SOURCE, "index.ts"),
      [RUNTIME]: path.join(root, "app.ts"),
    },
  }));
  assert.throws(() => getStreamingAsset(aliased), diagnostic("STREAM-B003"));
  const contaminated = await esbuild.build(options(root, {
    plugins: [{
      name: "contaminated-runtime",
      setup(build) {
        build.onLoad({ filter: /streaming-entry\.ts$/ }, () => ({
          contents: `import ${JSON.stringify(path.join(root, "app.ts"))}; console.log('altered-runtime');`,
          loader: "ts",
          resolveDir: SOURCE,
        }));
      },
    }, esbuildStreaming()],
  }));
  assert.throws(() => getStreamingAsset(contaminated), diagnostic("STREAM-B003"));
});

test("preserves required initialization when delegated resolution marks it side-effect-free", async t => {
  const root = await fixture(t);
  const result = await esbuild.build(options(root, {
    minify: true,
    define: { __WEBUI_DEV__: "false" },
    plugins: [
      esbuildStreaming(),
      {
        name: "side-effect-free-resolution",
        setup(build) {
          build.onResolve({ filter: /^@microsoft\/webui-framework\/streaming\.js$/ }, () => ({
            path: path.join(SOURCE, "streaming-entry.ts"), sideEffects: false,
          }));
        },
      },
    ],
  }));
  const asset = getStreamingAsset(result);
  assert.ok([asset.entry, ...asset.imports].some(id =>
    Object.entries(result.metafile!.outputs[id]!.inputs).some(([input, value]) =>
      input.endsWith("/streaming-entry.ts") && value.bytesInOutput > 0
    )
  ));
});

test("rejects a successful build whose coordinator initialization was removed by another plugin", async t => {
  const root = await fixture(t);
  const result = await esbuild.build(options(root, {
    plugins: [{
      name: "eliminated-entry",
      setup(build) {
        build.onResolve({ filter: /^@microsoft\/webui-framework\/streaming\.js$/ }, () => ({
          path: path.join(SOURCE, "streaming-entry.ts"), sideEffects: false,
        }));
      },
    }, esbuildStreaming()],
  }));
  assert.throws(() => getStreamingAsset(result), diagnostic("STREAM-B002"));
});

test("supports ordinary license banners and host-owned deployment mapping", async t => {
  const root = await fixture(t);
  const result = await esbuild.build(options(root, {
    publicPath: "https://cdn.example.test/site/v2",
    banner: { js: "/* Application license */" },
    footer: { js: "// end of bundle" },
    entryNames: "entry space/[name]-[hash]",
    chunkNames: "chunk space/[name]-[hash]",
  }));
  const asset = getStreamingAsset(result);
  assert.ok(asset.entry.startsWith("dist/entry space/"));
  for (const id of [asset.entry, ...asset.imports]) {
    assert.ok(result.metafile!.outputs[id]);
    assert.ok(result.outputFiles!.find(file => file.path === path.resolve(root, id))!.text
      .includes("/* Application license */"));
  }
  const error = new StreamingBuildError("STREAM-B001", "Invalid configuration", "Use browser ESM.");
  assert.equal(error.code, "STREAM-B001");
  assert.ok(error.message.includes("help:"));
});

test("requires a successful build before handing off coordinator output", async t => {
  const root = await fixture(t);
  const result = await esbuild.build(options(root));
  assert.throws(() => getStreamingAsset({
    ...result,
    errors: [{ id: "", pluginName: "later-validation", text: "rejected", location: null, notes: [], detail: undefined }],
  }), diagnostic("STREAM-B002"));
  assert.throws(() => getStreamingAsset({ ...result, metafile: undefined }), diagnostic("STREAM-B002"));
});

test("the public browser entry can be bundled directly without the optional adapter", async t => {
  const root = await fixture(t);
  const result = await esbuild.build(options(root, {
    entryPoints: { coordinator: RUNTIME, app: "app.ts" },
    plugins: [],
  }));
  assert.ok(Object.values(result.metafile!.outputs).some(output =>
    output.entryPoint?.endsWith("/streaming-entry.ts") && output.bytes > 0
  ));
  assert.equal(result.outputFiles!.some(file => file.path.endsWith(".json")), false);
});
