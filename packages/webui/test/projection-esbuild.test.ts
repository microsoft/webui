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

describe("esbuildProjection", () => {
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
