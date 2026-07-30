// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import * as path from "node:path";
import { describe, test } from "node:test";
import * as esbuild from "esbuild";
import {
  esbuildProjection,
  hashContent,
  validateManifestSchema,
} from "@microsoft/webui/projection.js";
import type {
  ProjectionManifest,
} from "@microsoft/webui/projection.js";

const FRAMEWORK_ENTRY = path.resolve(
  "..",
  "webui-framework",
  "src",
  "index.ts"
);

async function fixtureRoot(): Promise<string> {
  const root = await mkdtemp(
    path.join(process.cwd(), ".tmp-esbuild-projection-")
  );
  await mkdir(path.join(root, "src"), { recursive: true });
  await writeFile(
    path.join(root, "package.json"),
    JSON.stringify({ name: "projection-fixture" })
  );
  return root;
}

async function readManifest(
  root: string,
  outputDirectory = "dist"
): Promise<ProjectionManifest> {
  return JSON.parse(
    await readFile(
      path.join(root, outputDirectory, "webui-projection.json"),
      "utf8"
    )
  ) as ProjectionManifest;
}

function resolvedArtifact(
  root: string,
  manifest: ProjectionManifest,
  key: string
): string {
  return path.resolve(
    root,
    "dist",
    ...manifest.root.split("/"),
    ...key.split("/")
  );
}

async function writeCardFixture(root: string): Promise<void> {
  await writeFile(
    path.join(root, "src", "entry.ts"),
    "import('./card.ts');\n"
  );
  await writeFile(
    path.join(root, "src", "card.ts"),
    `
import { WebUIElement, observable, attr } from '@microsoft/webui-framework';
class Card extends WebUIElement {
  @observable value = '';
  @attr({ attribute: 'display-value' }) displayValue = '';
}
Card.define('probe-card');
`
  );
}

async function writeSharedChunkFixture(root: string): Promise<void> {
  // Two entries that both *statically* import one module. Splitting hoists it
  // into its own chunk that is named only inside each entry's bytes, so the
  // browser's preload scanner cannot see it — the case the closure exists for.
  await writeFile(
    path.join(root, "src", "shared.ts"),
    `
import { WebUIElement, observable } from '@microsoft/webui-framework';
export class Base extends WebUIElement {
  @observable shared = '';
}
export const padding = '${"padding".repeat(2048)}';
`
  );
  await writeFile(
    path.join(root, "src", "entry.ts"),
    `
import { Base, padding } from './shared.ts';
class Main extends Base {}
Main.define('probe-main');
export const used = padding.length;
`
  );
  await writeFile(
    path.join(root, "src", "island.ts"),
    `
import { Base, padding } from './shared.ts';
class Island extends Base {}
Island.define('probe-island');
export const used = padding.length;
`
  );
}

describe("esbuildProjection", () => {
  test("treats stdin as virtual even when sourcefile exists on disk", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeFile(
      path.join(root, "src", "entry.ts"),
      `
import { WebUIElement, observable } from '@microsoft/webui-framework';
class DiskCard extends WebUIElement {
  @observable diskOnly = '';
}
DiskCard.define('disk-card');
`
    );

    await esbuild.build({
      absWorkingDir: root,
      stdin: {
        contents: "export const fromStdin = true;\n",
        sourcefile: "src/entry.ts",
        resolveDir: root,
        loader: "ts",
      },
      outfile: "dist/index.js",
      bundle: true,
      format: "esm",
      write: true,
      plugins: [esbuildProjection()],
    });

    const manifest = await readManifest(root);
    assert.deepEqual(manifest.components, {});
  });

  test("emits a code-split manifest from the same esbuild run", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeCardFixture(root);

    const result = await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.ts"],
      outdir: "dist",
      bundle: true,
      splitting: true,
      format: "esm",
      write: true,
      alias: {
        "@microsoft/webui-framework": FRAMEWORK_ENTRY,
      },
      plugins: [esbuildProjection()],
    });

    assert.ok(result.metafile, "adapter should enable esbuild metafile");
    const manifest = await readManifest(root);
    assert.deepEqual(validateManifestSchema(manifest), []);
    assert.deepEqual(
      manifest.components["probe-card"]?.hydrationKeys,
      ["displayValue", "value"]
    );
    assert.deepEqual(
      manifest.components["probe-card"]?.navigationKeys,
      ["displayValue", "value"]
    );
    const componentOutputs =
      manifest.components["probe-card"]?.outputs ?? [];
    assert.equal(componentOutputs.length, 1);
    assert.ok(componentOutputs[0]!.includes("card-"));

    for (const [key, expectedHash] of Object.entries(
      manifest.outputs
    )) {
      const bytes = await readFile(
        resolvedArtifact(root, manifest, key)
      );
      assert.equal(hashContent(bytes), expectedHash);
    }
    const files = await readdir(path.join(root, "dist"));
    assert.equal(
      files.some((name) => name.includes(".tmp-")),
      false,
      "atomic manifest temporary files must be cleaned"
    );
  });

  test("records each entry's static import closure, largest first", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeSharedChunkFixture(root);

    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.ts", "src/island.ts"],
      outdir: "dist",
      bundle: true,
      splitting: true,
      format: "esm",
      write: true,
      alias: {
        "@microsoft/webui-framework": FRAMEWORK_ENTRY,
      },
      plugins: [esbuildProjection()],
    });

    const manifest = await readManifest(root);
    assert.deepEqual(validateManifestSchema(manifest), []);

    const closures = manifest.entryClosures ?? {};
    // esbuild also marks dynamic-import split points as entry points, so match
    // the two configured entries by name rather than counting keys.
    const entryKeys = Object.keys(closures).filter(
      (key) => key.endsWith("/entry.js") || key.endsWith("/island.js")
    );
    assert.equal(entryKeys.length, 2, "both entries should carry a closure");

    for (const entryKey of entryKeys) {
      const closure = closures[entryKey]!;
      assert.ok(
        closure.some((member) => member.includes("chunk-")),
        `${entryKey} must reach the shared chunk it statically imports`
      );
      assert.equal(
        closure.includes(entryKey),
        false,
        "an entry must not list itself"
      );
      for (const member of closure) {
        assert.ok(
          manifest.outputs[member] !== undefined,
          `closure member ${member} must be a declared output`
        );
      }

      // Preloads are issued in document order over a shared connection, so a
      // small chunk ahead of a large one delays the long pole. Verify the
      // contract against real bytes rather than trusting the sort.
      const sizes: number[] = [];
      for (const member of closure) {
        const bytes = await readFile(resolvedArtifact(root, manifest, member));
        sizes.push(bytes.byteLength);
      }
      for (let index = 1; index < sizes.length; index++) {
        assert.ok(
          sizes[index - 1]! >= sizes[index]!,
          `closure for ${entryKey} must be ordered largest-first`
        );
      }
    }
  });

  test("excludes dynamically imported chunks from the closure", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeCardFixture(root);

    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.ts"],
      outdir: "dist",
      bundle: true,
      splitting: true,
      format: "esm",
      write: true,
      alias: {
        "@microsoft/webui-framework": FRAMEWORK_ENTRY,
      },
      plugins: [esbuildProjection()],
    });

    const manifest = await readManifest(root);
    assert.deepEqual(validateManifestSchema(manifest), []);

    // `entry.ts` reaches `card.ts` only through `import()`. Preloading it would
    // defeat the deferral the author asked for, so its retained ownership
    // record must have an empty closure.
    const closures = manifest.entryClosures ?? {};
    const entryKey = Object.keys(closures).find((key) =>
      key.endsWith("/entry.js")
    );
    assert.ok(entryKey, "the configured entry must remain represented");
    const closure = closures[entryKey]!;
    assert.equal(closure.length, 0, "dynamic imports must not be preloaded");
  });

  test("suppresses closure members when publicPath changes served URLs", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeSharedChunkFixture(root);

    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.ts", "src/island.ts"],
      outdir: "dist",
      bundle: true,
      splitting: true,
      format: "esm",
      publicPath: "https://cdn.example.com/assets",
      write: true,
      alias: {
        "@microsoft/webui-framework": FRAMEWORK_ENTRY,
      },
      plugins: [esbuildProjection()],
    });

    const manifest = await readManifest(root);
    const closures = manifest.entryClosures ?? {};
    const configuredEntries = Object.entries(closures).filter(
      ([key]) => key.endsWith("/entry.js") || key.endsWith("/island.js")
    );
    assert.equal(configuredEntries.length, 2);
    for (const [, closure] of configuredEntries) {
      assert.deepEqual(
        closure,
        [],
        "local metafile paths must not become same-origin CDN preload guesses"
      );
    }
  });

  test("hashes esbuild outputFiles when write is false", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeCardFixture(root);

    const result = await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.ts"],
      outdir: "dist",
      bundle: true,
      splitting: true,
      format: "esm",
      write: false,
      alias: {
        "@microsoft/webui-framework": FRAMEWORK_ENTRY,
      },
      plugins: [esbuildProjection()],
    });

    const manifest = await readManifest(root);
    const expectedHashes = (result.outputFiles ?? [])
      .map((output) => hashContent(output.contents))
      .sort();
    assert.deepEqual(
      Object.values(manifest.outputs).sort(),
      expectedHashes
    );
    for (const key of Object.keys(manifest.outputs)) {
      await assert.rejects(
        access(resolvedArtifact(root, manifest, key))
      );
    }
  });

  test("uses resolved package identity instead of a literal package name", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeFile(
      path.join(root, "src", "fake-framework.ts"),
      `
export function observable(): void {}
export class WebUIElement {}
`
    );
    await writeFile(
      path.join(root, "src", "entry.ts"),
      `
import { observable, WebUIElement } from '@microsoft/webui-framework';
class NotWebUI extends WebUIElement { @observable value = ''; }
NotWebUI.define('not-webui-card');
`
    );

    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.ts"],
      outdir: "dist",
      bundle: true,
      write: true,
      alias: {
        "@microsoft/webui-framework": path.join(
          root,
          "src",
          "fake-framework.ts"
        ),
      },
      plugins: [esbuildProjection()],
    });

    const manifest = await readManifest(root);
    assert.deepEqual(manifest.components, {});
  });

  test("leaves the previous manifest intact when projection compilation fails", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeCardFixture(root);
    const options: esbuild.BuildOptions = {
      absWorkingDir: root,
      entryPoints: ["src/entry.ts"],
      outdir: "dist",
      bundle: true,
      write: true,
      alias: {
        "@microsoft/webui-framework": FRAMEWORK_ENTRY,
      },
      plugins: [esbuildProjection()],
    };

    await esbuild.build(options);
    const manifestPath = path.join(
      root,
      "dist",
      "webui-projection.json"
    );
    const before = await readFile(manifestPath, "utf8");
    await writeFile(
      path.join(root, "src", "card.ts"),
      `
import { WebUIElement } from '@microsoft/webui-framework';
const tag = 'probe-card';
class Card extends WebUIElement {}
Card.define(tag);
`
    );

    await assert.rejects(
      esbuild.build(options),
      (error: unknown) => {
        if (!(error instanceof Error) || !("errors" in error)) {
          return false;
        }
        return (
          error as { errors: Array<{ text: string }> }
        ).errors.some((entry) => entry.text.includes("PROJ-C008"));
      }
    );
    assert.equal(await readFile(manifestPath, "utf8"), before);
  });

  test("emits a separate fragment for an external component bundle", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeFile(
      path.join(root, "src", "entry.ts"),
      "import './shared-card.ts';\n"
    );
    await writeFile(
      path.join(root, "src", "shared-card.ts"),
      `
import { WebUIElement, observable } from '@microsoft/webui-framework';
export class SharedCard extends WebUIElement { @observable value = ''; }
SharedCard.define('shared-card');
`
    );

    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.ts"],
      outdir: "dist",
      bundle: true,
      write: true,
      external: ["./shared-card.ts"],
      plugins: [esbuildProjection()],
    });
    const appManifest = await readManifest(root);
    assert.deepEqual(appManifest.components, {});

    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/shared-card.ts"],
      outdir: "shared-dist",
      bundle: true,
      write: true,
      alias: {
        "@microsoft/webui-framework": FRAMEWORK_ENTRY,
      },
      plugins: [
        esbuildProjection({
          manifest: "shared-dist/webui-projection.json",
        }),
      ],
    });
    const sharedManifest = await readManifest(root, "shared-dist");
    assert.deepEqual(
      sharedManifest.components["shared-card"]?.hydrationKeys,
      ["value"]
    );
    assert.deepEqual(
      sharedManifest.components["shared-card"]?.navigationKeys,
      ["value"]
    );
  });
});
