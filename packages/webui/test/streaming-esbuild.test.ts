// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import { randomUUID } from "node:crypto";
import { access, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import * as path from "node:path";
import { gzipSync } from "node:zlib";
import { test, type TestContext } from "node:test";
import * as esbuild from "esbuild";
import { esbuildStreaming, StreamingBuildError, STREAMING_ASSETS_SCHEMA } from "@microsoft/webui/streaming.js";
import type { StreamingAssetsManifest } from "@microsoft/webui/streaming.js";
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

function descriptor(root: string, result: esbuild.BuildResult): StreamingAssetsManifest {
  const output = result.outputFiles?.find(file => file.path === path.join(root, "dist", "webui-streaming.json"));
  assert.ok(output);
  return JSON.parse(output.text) as StreamingAssetsManifest;
}

function diagnostic(code: string): (error: unknown) => boolean {
  return error => {
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
  const manifest = descriptor(root, result);
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
  assert.equal(manifest.schema, STREAMING_ASSETS_SCHEMA);
  assert.equal(manifest.coordinator.src, "https://cdn.example.test/assets/" + path.relative(path.join(root, "dist"), path.resolve(root, entry)).split(path.sep).join("/"));
  assert.equal(manifest.coordinator.type, "module");
  assert.equal(manifest.coordinator.async, true);
  const expected = [...early].filter(id => id !== entry).sort((a, b) =>
    meta.outputs[b]!.bytes - meta.outputs[a]!.bytes || Buffer.compare(Buffer.from(a), Buffer.from(b))
  ).map(id => "https://cdn.example.test/assets/" + path.relative(path.join(root, "dist"), path.resolve(root, id)).split(path.sep).join("/"));
  assert.deepEqual(manifest.coordinator.imports, expected);
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
    assert.deepEqual(descriptor(root, second), descriptor(root, third));
    assert.notDeepEqual(Object.keys(first.metafile!.outputs), Object.keys(second.metafile!.outputs));
  } finally {
    await context.dispose();
  }
});

test("coexists with projection and preserves the last disk manifest on build failure", async t => {
  const root = await fixture(t);
  const settings = options(root, {
    write: true,
    plugins: [esbuildProjection(), esbuildStreaming()],
  });
  await esbuild.build(settings);
  const filename = path.join(root, "dist", "webui-streaming.json");
  const previous = await readFile(filename, "utf8");
  assert.equal(JSON.parse(previous).schema, STREAMING_ASSETS_SCHEMA);
  await access(path.join(root, "dist", "webui-projection.json"));
  await writeFile(path.join(root, "app.ts"), "export const broken = ;");
  await assert.rejects(esbuild.build(settings));
  assert.equal(await readFile(filename, "utf8"), previous);
});

test("rejects configurations that introduce global code or duplicate module identities", async t => {
  const root = await fixture(t);
  for (const extra of [
    { format: "iife" as const },
    { splitting: false },
    { inject: [ENTRY] },
    { banner: { js: "console.log('application')" } },
    { preserveSymlinks: true },
    { plugins: [esbuildStreaming(), { name: "later", setup() {} }] },
  ]) {
    await assert.rejects(esbuild.build(options(root, extra)), diagnostic("STREAM-B001"));
  }
});

test("rejects external runtimes, wrong framework aliases and contaminated early chunks", async t => {
  const root = await fixture(t);
  await assert.rejects(esbuild.build(options(root, {
    external: [path.join(SOURCE, "streaming-entry.ts")],
  })), diagnostic("STREAM-B002"));
  await assert.rejects(esbuild.build(options(root, {
    alias: {
      "@microsoft/webui-framework": path.join(SOURCE, "index.ts"),
      [RUNTIME]: path.join(root, "app.ts"),
    },
  })), diagnostic("STREAM-B003"));
  await assert.rejects(esbuild.build(options(root, {
    plugins: [{
      name: "contaminated-runtime",
      setup(build) {
        build.onLoad({ filter: /streaming-entry\.ts$/ }, () => ({
          contents: `import ${JSON.stringify(path.join(root, "app.ts"))};`,
          loader: "ts",
          resolveDir: SOURCE,
        }));
      },
    }, esbuildStreaming()],
  })), diagnostic("STREAM-B003"));
});

test("does not overwrite a generated asset with the manifest", async t => {
  const root = await fixture(t);
  await assert.rejects(esbuild.build(options(root, {
    outExtension: { ".js": ".json" },
  })), diagnostic("STREAM-B004"));
});

test("validates asset delivery paths and exposes structured build errors", async t => {
  const root = await fixture(t);
  await assert.rejects(esbuild.build(options(root, { publicPath: "/assets?query" })), diagnostic("STREAM-B005"));
  await mkdir(path.join(root, "dist", "webui-streaming.json"), { recursive: true });
  await assert.rejects(esbuild.build(options(root, { write: true })), diagnostic("STREAM-B006"));
  const error = new StreamingBuildError("STREAM-B001", "Invalid configuration", "Use browser ESM.");
  assert.equal(error.code, "STREAM-B001");
  assert.ok(error.message.includes("help:"));
});
