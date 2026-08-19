// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import {
  cp,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { pathToFileURL } from "node:url";
import { describe, test } from "node:test";
import {
  esbuildProjection,
} from "@microsoft/webui/projection.js";
import type {
  OnStartResult,
  PluginBuild,
} from "esbuild";

interface ProjectionModule {
  compileProjection(context: unknown): Promise<unknown>;
}

interface TypeScriptVersionModule {
  supportedTypeScriptMajor(version: string): 6 | 7 | undefined;
}

const INVALID_CONTEXT = {
  graph: { modules: new Map(), entries: [] },
  membership: { outputs: new Map() },
  outputContents: new Map(),
  rootDir: ".",
  manifestPath: ".",
  bundlerName: "test",
  bundlerVersion: "1.0.0",
};

async function createProjectionFixture(
  typescriptVersion?: string,
  typescriptSource?: string
): Promise<{ readonly root: string; readonly module: ProjectionModule }> {
  const root = await mkdtemp(
    path.join(tmpdir(), "webui-projection-peer-")
  );
  await cp(
    path.resolve("dist", "projection"),
    path.join(root, "dist", "projection"),
    { recursive: true }
  );
  await writeFile(
    path.join(root, "package.json"),
    JSON.stringify({
      name: "projection-peer-fixture",
      version: "0.0.25",
      type: "module",
    })
  );
  if (typescriptSource !== undefined) {
    await mkdir(path.join(root, "node_modules"), { recursive: true });
    await cp(
      typescriptSource,
      path.join(root, "node_modules", "typescript"),
      { recursive: true }
    );
  } else if (typescriptVersion !== undefined) {
    const typescriptRoot = path.join(root, "node_modules", "typescript");
    await mkdir(typescriptRoot, { recursive: true });
    await writeFile(
      path.join(typescriptRoot, "package.json"),
      JSON.stringify({
        name: "typescript",
        version: typescriptVersion,
        type: "module",
        exports: "./index.js",
      })
    );
    await writeFile(
      path.join(typescriptRoot, "index.js"),
      `export const version = ${JSON.stringify(typescriptVersion)};\n`
    );
  }
  const module = (await import(
    pathToFileURL(
      path.join(root, "dist", "projection", "index.js")
    ).href
  )) as ProjectionModule;
  return { root, module };
}

describe("projection optional peers", () => {
  test("reports PROJ-P001 when TypeScript is absent", async () => {
    const { root, module } = await createProjectionFixture();
    try {
      await assert.rejects(
        module.compileProjection({}),
        (error: unknown) => {
          if (
            !(error instanceof Error) ||
            !("diagnostics" in error)
          ) {
            return false;
          }
          const diagnostics = (
            error as {
              diagnostics: Array<{ readonly code: string }>;
            }
          ).diagnostics;
          return diagnostics[0]?.code === "PROJ-P001";
        }
      );
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("accepts the supported TypeScript 7 peer", async () => {
    const module = (await import(
      "@microsoft/webui/projection.js"
    )) as ProjectionModule;
    await assert.rejects(
      module.compileProjection(INVALID_CONTEXT),
      (error: unknown) => {
        if (
          !(error instanceof Error) ||
          !("diagnostics" in error)
        ) {
          return false;
        }
        const diagnostics = (
          error as {
            diagnostics: Array<{ readonly code: string }>;
          }
        ).diagnostics;
        return diagnostics[0]?.code === "PROJ-C013";
      }
    );
  });

  test("keeps the TypeScript 6 lower bound supported", async () => {
    const { root, module } = await createProjectionFixture(
      undefined,
      path.resolve("node_modules", "typescript-6")
    );
    try {
      const moduleId = path.join(root, "src", "card.ts");
      const outputId = path.join(root, "dist", "card.js");
      const manifest = await module.compileProjection({
        graph: {
          modules: new Map([
            [
              moduleId,
              {
                id: moduleId,
                kind: "file",
                source: `
import { WebUIElement } from "@microsoft/webui-framework";
class Card extends WebUIElement {}
Card.define("ts6-card");
`,
                imports: [
                  {
                    specifier: "@microsoft/webui-framework",
                    resolvedId: undefined,
                    external: true,
                    kind: "static",
                    packageName: "@microsoft/webui-framework",
                  },
                ],
              },
            ],
          ]),
          entries: [moduleId],
        },
        membership: {
          outputs: new Map([[outputId, new Set([moduleId])]]),
        },
        outputContents: new Map([[outputId, "compiled"]]),
        rootDir: root,
        manifestPath: path.join(root, "dist", "projection.json"),
        bundlerName: "test",
        bundlerVersion: "1.0.0",
      }) as {
        readonly components: Record<string, unknown>;
      };
      assert.ok(manifest.components["ts6-card"]);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("rejects a TypeScript peer below the supported 7.x range", async () => {
    const { root, module } = await createProjectionFixture("7.0.1");
    try {
      await assert.rejects(
        module.compileProjection({}),
        (error: unknown) => {
          if (
            !(error instanceof Error) ||
            !("diagnostics" in error)
          ) {
            return false;
          }
          const diagnostics = (
            error as {
              diagnostics: Array<{ readonly code: string }>;
            }
          ).diagnostics;
          return diagnostics[0]?.code === "PROJ-P001";
        }
      );
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("rejects an untested TypeScript 7 version", async () => {
    const { root, module } = await createProjectionFixture("7.1.0");
    try {
      await assert.rejects(
        module.compileProjection(INVALID_CONTEXT),
        (error: unknown) => {
          if (
            !(error instanceof Error) ||
            !("diagnostics" in error)
          ) {
            return false;
          }
          const diagnostics = (
            error as {
              diagnostics: Array<{ readonly code: string }>;
            }
          ).diagnostics;
          return diagnostics[0]?.code === "PROJ-P001";
        }
      );
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("accepts SemVer build metadata without accepting prereleases", async () => {
    const versionPolicy = (await import(
      pathToFileURL(
        path.resolve("dist", "projection", "typescript-version.js")
      ).href
    )) as TypeScriptVersionModule;

    assert.equal(versionPolicy.supportedTypeScriptMajor("6.0.3+corp.1"), 6);
    assert.equal(versionPolicy.supportedTypeScriptMajor("7.0.2+corp.1"), 7);
    assert.equal(
      versionPolicy.supportedTypeScriptMajor("7.0.2-rc.1+corp.1"),
      undefined
    );
    assert.equal(
      versionPolicy.supportedTypeScriptMajor("7.0.2+corp..1"),
      undefined
    );
  });


  test("reports PROJ-P002 for an incompatible application esbuild", async () => {
    const plugin = esbuildProjection();
    let onStart:
      | (() => OnStartResult | null | void | Promise<OnStartResult | null | void>)
      | undefined;
    const initialOptions: Record<string, unknown> = {};
    const fakeBuild = {
      initialOptions,
      esbuild: { version: "0.27.0" },
      onStart(callback: typeof onStart) {
        onStart = callback;
      },
      onEnd() {},
    } as unknown as PluginBuild;

    await plugin.setup(fakeBuild);
    assert.equal(initialOptions["metafile"], true);
    assert.ok(onStart);
    const result = await onStart();
    assert.equal(result?.errors?.[0]?.id, "PROJ-P002");
  });

  test("root entry does not import projection tooling", async () => {
    const rootEntry = await readFile(
      path.resolve("dist", "index.js"),
      "utf8"
    );
    assert.equal(rootEntry.includes("/projection/"), false);
    assert.equal(rootEntry.includes("typescript"), false);
    assert.equal(rootEntry.includes("esbuild"), false);
  });
});
