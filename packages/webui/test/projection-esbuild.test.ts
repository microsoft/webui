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
import { pathToFileURL } from "node:url";
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

interface ConcurrencyModule {
  mapConcurrent<T, U>(
    values: ReadonlyArray<T>,
    maxConcurrency: number,
    operation: (value: T, index: number, workerIndex: number) => Promise<U>
  ): Promise<U[]>;
}

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
  test("bounds projection I/O concurrency and preserves order", async () => {
    const moduleUrl = new URL(
      "../projection/concurrency.js",
      import.meta.url
    );
    const { mapConcurrent } = (await import(
      moduleUrl.href
    )) as ConcurrencyModule;
    const values = Array.from({ length: 20 }, (_, index) => index);
    let active = 0;
    let peak = 0;

    const results = await mapConcurrent(
      values,
      4,
      async (value) => {
        active++;
        peak = Math.max(peak, active);
        await new Promise<void>((resolve) => setTimeout(resolve, 5));
        active--;
        return value * 2;
      }
    );

    assert.equal(peak, 4);
    assert.deepEqual(
      results,
      values.map((value) => value * 2)
    );
  });

  test("settles active projection I/O before propagating errors", async () => {
    const moduleUrl = new URL(
      "../projection/concurrency.js",
      import.meta.url
    );
    const { mapConcurrent } = (await import(
      moduleUrl.href
    )) as ConcurrencyModule;
    const started: number[] = [];
    let active = 0;

    await assert.rejects(
      mapConcurrent([0, 1, 2], 2, async (value) => {
        started.push(value);
        if (value === 1) throw new Error("read failed");
        active++;
        await new Promise<void>((resolve) => setTimeout(resolve, 10));
        active--;
        return value;
      }),
      /read failed/
    );

    assert.equal(active, 0);
    assert.deepEqual(started, [0, 1]);
  });

  test("proves opaque assets through emitted bytes", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    const weatherBytes = Uint8Array.from([
      0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46,
    ]);
    await writeFile(path.join(root, "src", "weather.jpg"), weatherBytes);
    await writeFile(
      path.join(root, "src", "entry.ts"),
      "import weather from './weather.jpg';\nexport { weather };\n"
    );

    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.ts"],
      outdir: "dist",
      bundle: true,
      format: "esm",
      write: true,
      loader: { ".jpg": "file" },
      plugins: [esbuildProjection()],
    });

    const manifest = await readManifest(root);
    assert.equal(
      Object.keys(manifest.inputs).some((key) =>
        key.endsWith("src/weather.jpg")
      ),
      false
    );
    assert.ok(
      Object.keys(manifest.inputs).some((key) =>
        key.endsWith("src/entry.ts")
      )
    );

    const outputKey = Object.keys(manifest.outputs).find((key) =>
      key.endsWith(".jpg")
    );
    assert.ok(outputKey);
    assert.equal(
      manifest.outputs[outputKey],
      hashContent(await readFile(resolvedArtifact(root, manifest, outputKey)))
    );
  });

  test("honors non-source loader overrides on source extensions", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    const assetBytes = Uint8Array.from([0xff, 0x00, 0xfe, 0x01]);
    await writeFile(path.join(root, "src", "asset.ts"), assetBytes);
    await writeFile(
      path.join(root, "src", "entry.js"),
      "import asset from './asset.ts';\nexport { asset };\n"
    );

    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.js"],
      outdir: "dist",
      bundle: true,
      format: "esm",
      write: true,
      loader: { ".ts": "file" },
      plugins: [esbuildProjection()],
    });

    const manifest = await readManifest(root);
    assert.equal(
      Object.keys(manifest.inputs).some((key) =>
        key.endsWith("src/asset.ts")
      ),
      false
    );
    assert.ok(
      Object.keys(manifest.inputs).some((key) =>
        key.endsWith("src/entry.js")
      )
    );
  });

  test("follows esbuild loader plugins and longest suffixes", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeFile(
      path.join(root, "src", "card.component.ts"),
      `
import { WebUIElement } from '@microsoft/webui-framework';
class Card extends WebUIElement {}
Card.define('suffix-card');
`
    );
    await writeFile(
      path.join(root, "src", "suffix-entry.js"),
      "import './card.component.ts';\n"
    );

    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/suffix-entry.js"],
      outdir: "suffix-dist",
      bundle: true,
      format: "esm",
      write: true,
      loader: { ".ts": "file", ".component.ts": "ts" },
      external: ["@microsoft/webui-framework"],
      plugins: [esbuildProjection()],
    });
    assert.ok(
      (await readManifest(root, "suffix-dist")).components["suffix-card"]
    );

    await writeFile(
      path.join(root, "src", "plugin-asset.ts"),
      Uint8Array.from([0xff, 0x00, 0xfe, 0x01])
    );
    await writeFile(
      path.join(root, "src", "plugin-entry.js"),
      "import asset from './plugin-asset.ts';\nexport { asset };\n"
    );
    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/plugin-entry.js"],
      outdir: "plugin-dist",
      bundle: true,
      format: "esm",
      write: true,
      plugins: [
        {
          name: "binary-ts-loader",
          setup(build) {
            build.onLoad(
              { filter: /plugin-asset\.ts$/ },
              async (args) => ({
                contents: await readFile(args.path),
                loader: "file",
              })
            );
          },
        },
        esbuildProjection(),
      ],
    });
    assert.equal(
      Object.keys(
        (await readManifest(root, "plugin-dist")).inputs
      ).some((key) => key.endsWith("src/plugin-asset.ts")),
      false
    );
  });

  test("resolves the default stdin loader from its sourcefile", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    const source = "const value: number = 1;\nexport { value };\n";

    await esbuild.build({
      absWorkingDir: root,
      stdin: {
        contents: source,
        sourcefile: "src/entry.ts",
        loader: "default",
        resolveDir: root,
      },
      outfile: "dist/index.js",
      bundle: true,
      format: "esm",
      write: true,
      plugins: [esbuildProjection()],
    });
    assert.equal(
      Object.keys((await readManifest(root)).inputs).length,
      1
    );

    await esbuild.build({
      absWorkingDir: root,
      stdin: {
        contents: source,
        sourcefile: "src/asset.ts",
        loader: "default",
        resolveDir: root,
      },
      outdir: "opaque-dist",
      bundle: true,
      format: "esm",
      write: true,
      loader: { ".ts": "file" },
      plugins: [esbuildProjection()],
    });
    assert.deepEqual(
      (await readManifest(root, "opaque-dist")).inputs,
      {}
    );
  });

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
    assert.deepEqual(manifest.components["probe-card"]?.attributes, {
      "display-value": { property: "displayValue", mode: 0 },
    });
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

  test("emits inherited exact aliases and modes without executing constructors", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeFile(path.join(root, "src", "base.ts"), `
import { WebUIElement, attr } from '@microsoft/webui-framework';
export class Base extends WebUIElement {
  @attr({ attribute: 'custom-attribute' }) unrelated = 'base';
  @attr({ mode: 'boolean' }) inherited = false;
}
`);
    await writeFile(path.join(root, "src", "entry.ts"), `
import { Base } from './base.ts';
import { attr, observable } from '@microsoft/webui-framework';
class Card extends Base {
  @attr ariaDescribedby = '';
  @attr({ attribute: 'derived-custom-attribute' }) unrelated = 'child';
  @observable inherited = true;
  @attr({ attribute: 'custom-boolean', mode: 'boolean' }) enabled = false;
  constructor() { super(); throw new Error('must not execute'); }
}
Card.define('test-card');
`);
    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.ts"],
      outdir: "dist",
      bundle: true,
      format: "esm",
      external: ["@microsoft/webui-framework"],
      tsconfigRaw: { compilerOptions: { experimentalDecorators: true } },
      plugins: [esbuildProjection()],
    });
    const manifest = await readManifest(root);
    assert.deepEqual(validateManifestSchema(manifest), []);
    assert.deepEqual(manifest.components["test-card"]?.attributes, {
      "aria-describedby": { property: "ariaDescribedby", mode: 0 },
      "custom-attribute": { property: "unrelated", mode: 0 },
      "custom-boolean": { property: "enabled", mode: 1 },
      "derived-custom-attribute": { property: "unrelated", mode: 0 },
      inherited: { property: "inherited", mode: 1 },
    });
  });

  test("rejects dynamic attr options instead of emitting guessed metadata", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeFile(path.join(root, "src", "entry.ts"), `
import { WebUIElement, attr } from '@microsoft/webui-framework';
const options = { attribute: 'custom-attribute' };
class Card extends WebUIElement { @attr(options) unrelated = ''; }
Card.define('test-card');
`);
    await assert.rejects(esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.ts"],
      outdir: "dist",
      bundle: true,
      format: "esm",
      external: ["@microsoft/webui-framework"],
      tsconfigRaw: { compilerOptions: { experimentalDecorators: true } },
      logLevel: "silent",
      plugins: [esbuildProjection()],
    }), /PROJ-C007/);
    await assert.rejects(access(path.join(root, "dist", "webui-projection.json")));
  });

  test("resolves inherited alias overrides by registration rather than property order", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeFile(path.join(root, "src", "entry.ts"), `
import { WebUIElement, attr } from '@microsoft/webui-framework';
class Base extends WebUIElement {
  @attr({ attribute: 'shared-name' }) zBase = '';
}
class Card extends Base {
  @attr({ attribute: 'shared-name', mode: 'boolean' }) aDerived = false;
}
Card.define('test-card');
`);
    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.ts"],
      outdir: "dist",
      bundle: true,
      format: "esm",
      external: ["@microsoft/webui-framework"],
      tsconfigRaw: { compilerOptions: { experimentalDecorators: true } },
      logLevel: "silent",
      plugins: [esbuildProjection()],
    });
    const manifest = await readManifest(root);
    assert.deepEqual(validateManifestSchema(manifest), []);
    assert.deepEqual(manifest.components["test-card"]?.attributes, {
      "shared-name": { property: "aDerived", mode: 1 },
    });
    assert.deepEqual(manifest.components["test-card"]?.hydrationKeys, ["aDerived", "zBase"]);
  });

  test("matches actual inbound decorators with experimental TypeScript registration order", async (t) => {
    const root = await fixtureRoot();
    t.after(() => rm(root, { recursive: true, force: true }));
    const frameworkStub = path.join(root, "src", "framework.ts");
    const decoratorsPath = path.join(path.dirname(FRAMEWORK_ENTRY), "decorators.ts");
    await writeFile(frameworkStub, `
export { attr, observable } from ${JSON.stringify(decoratorsPath)};
export class WebUIElement { static define(_tag: string) {} }
`);
    await writeFile(path.join(root, "src", "entry.ts"), `
import { WebUIElement, attr, observable } from '@microsoft/webui-framework';
class Base extends WebUIElement {
  @attr({ attribute: 'old-expanded', mode: 'boolean' }) expanded = false;
  @attr({ attribute: 'same-alias', mode: 'boolean' }) same = false;
  @attr({ attribute: 'shared-target', mode: 'boolean' }) zBase = false;
  @attr({ attribute: 'retained', mode: 'boolean' }) retained = false;
}
export class Card extends Base {
  @attr({ attribute: 'new-expanded' }) expanded = '';
  @attr({ attribute: 'same-alias' }) same = '';
  @attr({ attribute: 'shared-target' }) aDerived = '';
  @observable retained = false;
  @attr({ attribute: 'own-target' }) zFirst = '';
  @attr({ attribute: 'own-target', mode: 'boolean' }) aLast = false;
  @attr({ attribute: 'stack-first', mode: 'boolean' })
  @attr({ attribute: 'stack-second' })
  stacked = '';
  @attr({ attribute: 'stack-same', mode: 'boolean' })
  @attr({ attribute: 'stack-same' })
  stackedSame = false;
}
Card.define('test-card');
`);
    await esbuild.build({
      absWorkingDir: root,
      entryPoints: ["src/entry.ts"],
      outdir: "dist",
      outExtension: { ".js": ".mjs" },
      bundle: true,
      format: "esm",
      target: "es2020",
      alias: { "@microsoft/webui-framework": frameworkStub },
      tsconfigRaw: {
        compilerOptions: {
          experimentalDecorators: true,
          useDefineForClassFields: false,
        },
      },
      plugins: [esbuildProjection()],
    });
    const manifest = await readManifest(root);
    const component = manifest.components["test-card"]!;
    assert.deepEqual(component.attributes, {
      "new-expanded": { property: "expanded", mode: 0 },
      "old-expanded": { property: "expanded", mode: 1 },
      "own-target": { property: "aLast", mode: 1 },
      retained: { property: "retained", mode: 1 },
      "same-alias": { property: "same", mode: 0 },
      "shared-target": { property: "aDerived", mode: 0 },
      "stack-first": { property: "stacked", mode: 1 },
      "stack-same": { property: "stackedSame", mode: 1 },
      "stack-second": { property: "stacked", mode: 0 },
    });
    assert.deepEqual(component.hydrationKeys, [
      "aDerived", "aLast", "expanded", "retained", "same", "stacked",
      "stackedSame", "zBase", "zFirst",
    ]);
    assert.deepEqual(component.navigationKeys, component.hydrationKeys);
    assert.deepEqual(validateManifestSchema(manifest), []);

    interface DecoratedInstance {
      [property: string]: unknown;
      attributeChangedCallback(
        name: string,
        oldValue: string | null,
        newValue: string | null
      ): void;
    }
    const { Card }: {
      Card: { new(): DecoratedInstance; readonly observedAttributes: string[] };
    } = await import(pathToFileURL(path.join(root, "dist", "entry.mjs")).href);
    assert.deepEqual(
      [...new Set(Card.observedAttributes)].sort(),
      Object.keys(component.attributes!)
    );
    for (const [attribute, definition] of Object.entries(component.attributes!)) {
      const instance = new Card();
      const before: ReadonlyMap<string, unknown> = new Map(
        component.hydrationKeys.map((key) => [key, instance[key]])
      );
      instance.attributeChangedCallback(attribute, null, "present");
      for (const key of component.hydrationKeys) {
        assert.equal(
          instance[key],
          key === definition.property
            ? definition.mode === 1 ? true : "present"
            : before.get(key),
          `${attribute} must route only to ${definition.property}`
        );
      }
      instance.attributeChangedCallback(attribute, "present", null);
      assert.equal(instance[definition.property], definition.mode === 1 ? false : null);
    }
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
