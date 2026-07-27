// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import * as path from "node:path";
import { describe, test } from "node:test";
import {
  compileProjection,
  computeBuildId,
  serializeManifestCanonical,
  validateManifestSchema,
} from "@microsoft/webui/projection.js";
import type {
  AdapterContext,
  ModuleNode,
  ResolvedImport,
} from "@microsoft/webui/projection.js";

const ROOT = path.resolve(".webui-projection-unit");

function id(relative: string): string {
  return path.join(ROOT, relative);
}

function frameworkEdge(): ResolvedImport {
  return {
    specifier: "@microsoft/webui-framework",
    resolvedId: undefined,
    external: true,
    kind: "static",
    packageName: "@microsoft/webui-framework",
  };
}

function context(modules: ReadonlyArray<ModuleNode>): AdapterContext {
  const outputId = id("dist/index.js");
  return {
    graph: {
      modules: new Map(modules.map((module) => [module.id, module])),
      entries: [modules[modules.length - 1]!.id],
    },
    membership: {
      outputs: new Map([
        [outputId, new Set(modules.map((module) => module.id))],
      ]),
    },
    outputContents: new Map([[outputId, "compiled-output"]]),
    rootDir: ROOT,
    manifestPath: id("dist/webui-projection.json"),
    bundlerName: "test",
    bundlerVersion: "1.0.0",
  };
}

describe("projection compiler semantics", () => {
  test("uses adapter-resolved targets instead of reconstructing extensions", async () => {
    const baseId = id("src/base.ts");
    const derivedId = id("src/derived.ts");
    const manifest = await compileProjection(
      context([
        {
          id: baseId,
          kind: "file",
          source: `
import { observable, WebUIElement } from '@microsoft/webui-framework';
export class Base extends WebUIElement { @observable baseValue = ''; }
`,
          imports: [frameworkEdge()],
        },
        {
          id: derivedId,
          kind: "file",
          source: `
import { Base } from './base.js';
class Derived extends Base {}
Derived.define('resolved-card');
`,
          imports: [
            {
              specifier: "./base.js",
              resolvedId: baseId,
              external: false,
              kind: "static",
            },
          ],
        },
      ])
    );

    assert.deepEqual(
      manifest.components["resolved-card"]?.hydrationKeys,
      ["baseValue"]
    );
    assert.deepEqual(
      manifest.components["resolved-card"]?.navigationKeys,
      ["baseValue"]
    );
  });

  test("ignores known non-framework property decorators", async () => {
    const cardId = id("src/decorated.ts");
    const manifest = await compileProjection(
      context([
        {
          id: cardId,
          kind: "file",
          source: `
import { observable, WebUIElement } from '@microsoft/webui-framework';
function localDecorator(_target: object, _name: string): void {}
class Decorated extends WebUIElement {
  @localDecorator
  @observable value = '';
}
Decorated.define('decorated-card');
`,
          imports: [frameworkEdge()],
        },
      ])
    );

    assert.deepEqual(
      manifest.components["decorated-card"]?.hydrationKeys,
      ["value"]
    );
    assert.deepEqual(
      manifest.components["decorated-card"]?.navigationKeys,
      ["value"]
    );
  });

  test("ignores a locally shadowed customElements registry", async () => {
    const moduleId = id("src/local-registry.ts");
    const manifest = await compileProjection(
      context([
        {
          id: moduleId,
          kind: "file",
          source: `
import { WebUIElement } from '@microsoft/webui-framework';
class Card extends WebUIElement {}
const customElements = { define() {} };
customElements.define('not-global-card', Card);
`,
          imports: [frameworkEdge()],
        },
      ])
    );

    assert.deepEqual(manifest.components, {});
  });
});

describe("projection manifest hashing", () => {
  test("matches the cross-language canonical build-ID vector", () => {
    const buildId = computeBuildId({
      producerName: "@microsoft/webui/projection.js",
      producerVersion: "0.0.18",
      adapterName: "esbuild",
      adapterBundler: "esbuild@0.28.1",
      root: "..",
      analysisHash: `sha256:${"1".repeat(64)}`,
      sortedInputs: [["src/a.ts", `sha256:${"2".repeat(64)}`]],
      sortedOutputs: [["dist/a.js", `sha256:${"3".repeat(64)}`]],
      sortedComponents: [
        [
          "a-card",
          "src/a.ts",
          ["dist/a.js"],
          ["displayValue"],
          ["displayValue", "é"],
        ],
      ],
    });

    assert.equal(
      buildId,
      "sha256:8319202a060626c39cce76df50197c92dee27aab29d601161183c188204d7c18"
    );
  });

  test("component output membership changes the build ID", () => {
    const common = {
      producerName: "@microsoft/webui/projection.js",
      producerVersion: "0.0.18",
      adapterName: "esbuild",
      adapterBundler: "esbuild@0.28.1",
      root: "..",
      analysisHash: `sha256:${"1".repeat(64)}`,
      sortedInputs: [] as const,
      sortedOutputs: [
        ["dist/a.js", `sha256:${"2".repeat(64)}`],
        ["dist/b.js", `sha256:${"3".repeat(64)}`],
      ] as const,
    };
    const first = computeBuildId({
      ...common,
      sortedComponents: [
        [
          "a-card",
          "src/a.ts",
          ["dist/a.js"],
          ["value"],
          ["value"],
        ],
      ],
    });
    const second = computeBuildId({
      ...common,
      sortedComponents: [
        [
          "a-card",
          "src/a.ts",
          ["dist/b.js"],
          ["value"],
          ["value"],
        ],
      ],
    });

    assert.notEqual(first, second);
  });

  test("canonical serialization fixes top-level and map order", () => {
    const json = serializeManifestCanonical({
      schema: "webui.state-projection/v1",
      producer: {
        name: "@microsoft/webui/projection.js",
        version: "1.0.0",
      },
      adapter: { name: "test", bundler: "test@1.0.0" },
      root: "..",
      analysisHash: `sha256:${"1".repeat(64)}`,
      buildId: `sha256:${"2".repeat(64)}`,
      outputs: {
        "dist/z.js": `sha256:${"3".repeat(64)}`,
        "dist/a.js": `sha256:${"4".repeat(64)}`,
      },
      inputs: {
        "src/z.ts": `sha256:${"5".repeat(64)}`,
        "src/a.ts": `sha256:${"6".repeat(64)}`,
      },
      components: {},
    });

    assert.ok(json.indexOf('"dist/a.js"') < json.indexOf('"dist/z.js"'));
    assert.ok(json.indexOf('"src/a.ts"') < json.indexOf('"src/z.ts"'));
  });

  test("rejects virtual hashes on physical disk paths", () => {
    const errors = validateManifestSchema({
      schema: "webui.state-projection/v1",
      producer: {
        name: "@microsoft/webui/projection.js",
        version: "1.0.0",
      },
      adapter: { name: "test", bundler: "test@1.0.0" },
      root: "..",
      analysisHash: `sha256:${"1".repeat(64)}`,
      buildId: `sha256:${"2".repeat(64)}`,
      outputs: { "dist/index.js": "virtual" },
      inputs: {},
      components: {},
    });

    assert.deepEqual(errors, ["PROJ-S004"]);
  });
});
