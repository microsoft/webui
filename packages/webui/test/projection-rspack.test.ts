// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import {
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import * as path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { describe, test } from "node:test";
import {
  experiments,
  rspack,
} from "@rspack/core";
import type {
  Compiler,
  Configuration,
  Stats,
} from "@rspack/core";
import {
  hashContent,
  validateManifestSchema,
} from "@microsoft/webui/projection/core.js";
import type {
  AdapterContext,
  ProjectionManifest,
} from "@microsoft/webui/projection/core.js";
import {
  rspackProjection,
} from "@microsoft/webui/projection/rspack.js";
import type {
  RspackProjectionPlugin,
} from "@microsoft/webui/projection/rspack.js";

interface Fixture {
  readonly root: string;
  readonly entryPath: string;
  readonly virtualPath: string;
}

interface CapturedProjection {
  context: AdapterContext | undefined;
  callbackComplete: boolean;
  callbackCount: number;
  rawBaseDependencies: number;
  sawConcatenation: boolean;
  targetlessDependencyTypes: string[];
}

async function createFixture(): Promise<Fixture> {
  const root = await mkdtemp(
    path.join(process.cwd(), ".tmp-rspack-projection-")
  );
  const sourceDirectory = path.join(root, "src");
  const frameworkDirectory = path.join(root, "framework");
  await Promise.all([
    mkdir(sourceDirectory, { recursive: true }),
    mkdir(frameworkDirectory, { recursive: true }),
  ]);
  await Promise.all([
    writeFile(
      path.join(root, "package.json"),
      JSON.stringify({
        name: "rspack-projection-fixture",
        sideEffects: ["*.css"],
      })
    ),
    writeFile(
      path.join(frameworkDirectory, "package.json"),
      JSON.stringify({
        name: "@microsoft/webui-framework",
        type: "module",
      })
    ),
    writeFile(
      path.join(frameworkDirectory, "index.ts"),
      `
export function observable(): void {}
export function attr(): () => void { return () => {}; }
export class WebUIElement {
  static define(_name: string): void {}
}
`
    ),
    writeFile(
      path.join(sourceDirectory, "base.ts"),
      `
import { WebUIElement, observable } from '@microsoft/webui-framework';
export class BaseCard extends WebUIElement {
  @observable baseValue = '';
}
`
    ),
    writeFile(
      path.join(sourceDirectory, "lazy.ts"),
      `
import { WebUIElement, attr } from '@microsoft/webui-framework';
class LazyCard extends WebUIElement {
  @attr({ attribute: 'lazy-value' }) lazyValue = '';
}
LazyCard.define('lazy-card');
`
    ),
    writeFile(
      path.join(sourceDirectory, "barrel.ts"),
      `
import { importValue } from './pruned-import';
export { importValue };
export * from './pruned-export';
export { specifierValue } from './pruned-specifier';
export const keptValue = 'kept';
`
    ),
    writeFile(
      path.join(sourceDirectory, "pruned-import.ts"),
      "export const importValue = 'unused';\n"
    ),
    writeFile(
      path.join(sourceDirectory, "pruned-export.ts"),
      "export const exportValue = 'unused';\n"
    ),
    writeFile(
      path.join(sourceDirectory, "pruned-specifier.ts"),
      "export const specifierValue = 'unused';\n"
    ),
  ]);
  const entryPath = path.join(sourceDirectory, "entry.ts");
  await writeValidEntry(entryPath, "mainValue");
  return {
    root,
    entryPath,
    virtualPath: path.join(sourceDirectory, "virtual.ts"),
  };
}

async function writeValidEntry(
  entryPath: string,
  propertyName: string
): Promise<void> {
  await writeFile(
    entryPath,
    `
import externalValue from 'external-lib';
import { observable } from '@microsoft/webui-framework';
import { BaseCard } from './base';
import { keptValue } from './barrel';
import { virtualValue } from './virtual';

class MainCard extends BaseCard {
  @observable ${propertyName} = '';
}
MainCard.define('main-card');
console.log(externalValue, keptValue, virtualValue);
void import('./lazy');
`
  );
}

function createConfiguration(
  fixture: Fixture,
  plugin: RspackProjectionPlugin
): Configuration {
  return {
    context: fixture.root,
    mode: "production",
    target: "web",
    entry: "./src/entry.ts",
    devtool: false,
    cache: false,
    externals: {
      "external-lib": "commonjs external-lib",
    },
    resolve: {
      extensions: [".ts", ".js"],
      alias: {
        "@microsoft/webui-framework": path.join(
          fixture.root,
          "framework",
          "index.ts"
        ),
      },
    },
    module: {
      rules: [
        {
          test: /\.ts$/,
          use: [
            {
              loader: "builtin:swc-loader",
              options: {
                jsc: {
                  parser: {
                    syntax: "typescript",
                    decorators: true,
                  },
                  transform: {
                    legacyDecorator: true,
                  },
                },
              },
            },
          ],
        },
      ],
    },
    optimization: {
      concatenateModules: true,
      minimize: false,
    },
    output: {
      path: path.join(fixture.root, "dist"),
      filename: "[name].js",
      chunkFilename: "[name]-[contenthash].js",
      clean: true,
    },
    plugins: [
      new experiments.VirtualModulesPlugin({
        [fixture.virtualPath]: "export const virtualValue = 'virtual';\n",
      }),
      plugin,
    ],
  };
}

function runCompiler(
  compiler: Compiler,
  modifiedFile?: string
): Promise<Stats> {
  return new Promise((resolve, reject) => {
    const callback = (error: Error | null, stats?: Stats): void => {
      if (error) {
        reject(error);
      } else if (!stats) {
        reject(new Error("Rspack returned no stats"));
      } else {
        resolve(stats);
      }
    };
    if (modifiedFile === undefined) {
      compiler.run(callback);
    } else {
      compiler.run(callback, {
        modifiedFiles: new Set([modifiedFile]),
      });
    }
  });
}

function closeCompiler(compiler: Compiler): Promise<void> {
  return new Promise((resolve, reject) => {
    compiler.close((error) => {
      if (error) reject(error);
      else resolve();
    });
  });
}

function manifestPath(root: string): string {
  return path.join(root, "dist", "webui-projection.json");
}

async function readManifest(root: string): Promise<ProjectionManifest> {
  return JSON.parse(
    await readFile(manifestPath(root), "utf8")
  ) as ProjectionManifest;
}

function findOutput(
  context: AdapterContext,
  predicate: (name: string) => boolean
): string {
  const output = [...context.membership.outputs.keys()].find((value) =>
    predicate(path.basename(value))
  );
  assert.ok(output, "expected Rspack output was not present");
  return output;
}

describe("rspackProjection", () => {
  test("normalizes the production graph and awaits post-manifest work", async (t) => {
    const fixture = await createFixture();
    t.after(() => rm(fixture.root, { recursive: true, force: true }));
    const captured: CapturedProjection = {
      context: undefined,
      callbackComplete: false,
      callbackCount: 0,
      rawBaseDependencies: 0,
      sawConcatenation: false,
      targetlessDependencyTypes: [],
    };
    const plugin = rspackProjection({
      async afterManifest(result) {
        captured.callbackCount++;
        captured.context = result.context;
        captured.sawConcatenation = [
          ...result.compilation.modules,
        ].some(
          (module) =>
            module instanceof
            result.compiler.rspack.ConcatenatedModule
        );
        const rawEntry = [...result.compilation.modules].find(
          (module) =>
            module instanceof result.compiler.rspack.NormalModule &&
            path.resolve(module.resource) === fixture.entryPath
        );
        captured.rawBaseDependencies =
          rawEntry?.dependencies.filter(
            (dependency) => dependency.request === "./base"
          ).length ?? 0;
        const rawBarrel = [...result.compilation.modules].find(
          (module) =>
            module instanceof result.compiler.rspack.NormalModule &&
            path.basename(module.resource) === "barrel.ts"
        );
        captured.targetlessDependencyTypes =
          rawBarrel?.dependencies
            .filter((dependency) => {
              const target =
                result.compilation.moduleGraph.getModule(dependency) ??
                result.compilation.moduleGraph.getResolvedModule(dependency);
              return (
                dependency.request?.startsWith("./pruned-") === true &&
                (target === undefined || target === null)
              );
            })
            .map((dependency) => dependency.type) ?? [];
        const installed = await readManifest(fixture.root);
        assert.equal(installed.buildId, result.manifest.buildId);
        for (const [outputId, contents] of result.context.outputContents) {
          assert.deepEqual(
            Buffer.from(contents),
            await readFile(outputId)
          );
        }
        await delay(20);
        captured.callbackComplete = true;
      },
    });
    const compiler = rspack(createConfiguration(fixture, plugin));
    t.after(() => closeCompiler(compiler));

    const stats = await runCompiler(compiler);
    assert.equal(stats.hasErrors(), false, stats.toString({ errors: true }));
    assert.equal(captured.callbackComplete, true);
    assert.equal(captured.callbackCount, 1);
    assert.equal(captured.sawConcatenation, true);
    assert.ok(captured.rawBaseDependencies >= 2);
    for (const type of [
      "esm import",
      "esm export import",
      "esm export import specifier",
    ]) {
      assert.ok(
        captured.targetlessDependencyTypes.includes(type),
        `expected a targetless ${type} dependency; saw ${captured.targetlessDependencyTypes.join(", ")}`
      );
    }
    const context = captured.context;
    assert.ok(context);

    const entryId = path.resolve(fixture.root, "src", "entry.ts");
    const barrelId = path.resolve(fixture.root, "src", "barrel.ts");
    const baseId = path.resolve(fixture.root, "src", "base.ts");
    const lazyId = path.resolve(fixture.root, "src", "lazy.ts");
    const entry = context.graph.modules.get(entryId);
    assert.ok(entry);
    const baseEdges = entry.imports.filter(
      (edge) => edge.specifier === "./base"
    );
    assert.equal(baseEdges.length, 1);
    assert.equal(baseEdges[0]?.kind, "static");
    assert.equal(baseEdges[0]?.resolvedId, baseId);
    assert.equal(baseEdges[0]?.external, false);
    const barrel = context.graph.modules.get(barrelId);
    assert.ok(barrel);
    assert.equal(
      barrel.imports.some((edge) => edge.specifier.startsWith("./pruned-")),
      false
    );

    const frameworkEdge = entry.imports.find(
      (edge) => edge.specifier === "@microsoft/webui-framework"
    );
    assert.equal(
      frameworkEdge?.packageName,
      "@microsoft/webui-framework"
    );
    const externalEdge = entry.imports.find(
      (edge) => edge.specifier === "external-lib"
    );
    assert.equal(externalEdge?.external, true);
    assert.equal(externalEdge?.resolvedId, undefined);
    assert.equal(externalEdge?.packageName, "external-lib");
    const lazyEdge = entry.imports.find(
      (edge) => edge.specifier === "./lazy"
    );
    assert.equal(lazyEdge?.kind, "dynamic");
    assert.equal(lazyEdge?.resolvedId, lazyId);

    const virtualId = [...context.graph.modules.keys()].find(
      (id) =>
        id.startsWith("\0rspack:") &&
        id.endsWith("src/virtual.ts")
    );
    assert.ok(virtualId, "expected a normalized virtual module id");
    assert.equal(virtualId, "\0rspack:src/virtual.ts");
    assert.equal(virtualId.includes(fixture.root), false);

    const mainOutput = findOutput(
      context,
      (name) => name === "main.js"
    );
    const lazyOutput = findOutput(
      context,
      (name) => name !== "main.js" && name.endsWith(".js")
    );
    assert.ok(context.membership.outputs.get(mainOutput)?.has(entryId));
    assert.ok(context.membership.outputs.get(mainOutput)?.has(baseId));
    assert.ok(context.membership.outputs.get(lazyOutput)?.has(lazyId));
    assert.equal(
      context.membership.outputs.get(mainOutput)?.has(lazyId),
      false
    );

    const manifest = await readManifest(fixture.root);
    assert.deepEqual(validateManifestSchema(manifest), []);
    assert.deepEqual(
      manifest.components["main-card"]?.hydrationKeys,
      ["baseValue", "mainValue"]
    );
    assert.deepEqual(
      manifest.components["lazy-card"]?.hydrationKeys,
      ["lazyValue"]
    );
    assert.equal(
      manifest.components["main-card"]?.outputs.some((output) =>
        output.endsWith("/main.js")
      ),
      true
    );
    assert.equal(
      manifest.components["lazy-card"]?.outputs.some((output) =>
        output.endsWith("/main.js")
      ),
      false
    );
    for (const [key, expectedHash] of Object.entries(manifest.outputs)) {
      const outputPath = path.resolve(
        path.dirname(manifestPath(fixture.root)),
        ...manifest.root.split("/"),
        ...key.split("/")
      );
      assert.equal(hashContent(await readFile(outputPath)), expectedHash);
    }
  });

  test("preserves a valid manifest through clean failures and recovers", async (t) => {
    const fixture = await createFixture();
    t.after(() => rm(fixture.root, { recursive: true, force: true }));
    let callbackCount = 0;
    const plugin = rspackProjection({
      afterManifest(result) {
        assert.equal(
          result.compilation.getAsset("webui-projection.json"),
          undefined
        );
        callbackCount++;
      },
    });
    const compiler = rspack(createConfiguration(fixture, plugin));
    t.after(() => closeCompiler(compiler));

    const initial = await runCompiler(compiler);
    assert.equal(initial.hasErrors(), false);
    const before = await readFile(manifestPath(fixture.root), "utf8");
    assert.equal(callbackCount, 1);

    await writeFile(
      fixture.entryPath,
      `
import { BaseCard } from './base';
const tag = 'main-card';
class MainCard extends BaseCard {}
MainCard.define(tag);
`
    );
    const failed = await runCompiler(compiler, fixture.entryPath);
    assert.equal(failed.hasErrors(), true);
    assert.match(failed.toString({ errors: true }), /PROJ-C008/);
    assert.equal(
      await readFile(manifestPath(fixture.root), "utf8"),
      before
    );
    assert.equal(callbackCount, 1);

    await writeValidEntry(fixture.entryPath, "recoveredValue");
    const recovered = await runCompiler(compiler, fixture.entryPath);
    assert.equal(
      recovered.hasErrors(),
      false,
      recovered.toString({ errors: true })
    );
    assert.deepEqual(
      (await readManifest(fixture.root)).components["main-card"]
        ?.hydrationKeys,
      ["baseValue", "recoveredValue"]
    );
    assert.equal(callbackCount, 2);
  });

  test("reports a rejected post-manifest callback as a compilation error", async (t) => {
    const fixture = await createFixture();
    t.after(() => rm(fixture.root, { recursive: true, force: true }));
    const compiler = rspack(
      createConfiguration(
        fixture,
        rspackProjection({
          afterManifest() {
            throw new Error("protocol rebuild failed");
          },
        })
      )
    );
    t.after(() => closeCompiler(compiler));

    const stats = await runCompiler(compiler);
    assert.equal(stats.hasErrors(), true);
    assert.match(
      stats.toString({ errors: true }),
      /protocol rebuild failed/
    );
    assert.equal(
      (await readManifest(fixture.root)).schema,
      "webui.state-projection/v1"
    );
  });
});
