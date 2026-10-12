// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import * as path from "node:path";
import { test } from "node:test";
import { API } from "typescript/unstable/sync";
import { compileProjection } from "@microsoft/webui/projection.js";
import type { AdapterContext, ModuleNode, ResolvedImport } from "@microsoft/webui/projection.js";

interface ProjectionParser {
  prepare(moduleIds: readonly string[]): void;
  parse(moduleId: string): { sourceFile: { text: string }; hasDiagnostics: boolean } | undefined;
  close(): void;
}

const { createProjectionParser } = await import(
  new URL("../projection/typescript-api.js", import.meta.url).href
) as { createProjectionParser(modules: ReadonlyMap<string, ModuleNode>): ProjectionParser };

const ROOT = path.resolve(".projection-parser-test");
const id = (name: string): string => path.join(ROOT, name);
const framework: ResolvedImport = {
  specifier: "@microsoft/webui-framework", resolvedId: undefined,
  external: true, kind: "static", packageName: "@microsoft/webui-framework",
};

function source(name: string, text: string, imports: readonly ResolvedImport[] = []): ModuleNode {
  return { id: id(name), kind: "file", source: text, imports };
}

function context(modules: ModuleNode[]): AdapterContext {
  return {
    graph: { modules: new Map(modules.map((module) => [module.id, module])), entries: [modules[0]!.id] },
    membership: { outputs: new Map([[id("dist/index.js"), new Set(modules.map((module) => module.id))]]) },
    outputContents: new Map([[id("dist/index.js"), "compiled"]]),
    rootDir: ROOT, manifestPath: id("dist/projection.json"),
    bundlerName: "test", bundlerVersion: "1.0.0",
  };
}

test("batches candidate files while keeping dependencies and unused files lazy", async (t) => {
  const requests: Array<Parameters<typeof API.prototype.updateSnapshot>[0]> = [];
  const original = API.prototype.updateSnapshot;
  t.mock.method(API.prototype, "updateSnapshot", function (
    this: API,
    options: Parameters<typeof API.prototype.updateSnapshot>[0]
  ) {
    requests.push(structuredClone(options));
    return original.call(this, options);
  });
  const clear = t.mock.method(API.prototype, "clearSourceFileCache");
  const components = Array.from({ length: 20 }, (_, index) => source(`card-${index}.ts`, `
import { Base } from './base.js';
class Card extends Base {}
Card.define('card-${index}');
`, [{ specifier: "./base.js", resolvedId: id("base.ts"), external: false, kind: "static" }]));
  const manifest = await compileProjection(context([
    ...components,
    source("base.ts", `
import { WebUIElement, observable } from '@microsoft/webui-framework';
export class Base extends WebUIElement { @observable value = ''; }
`, [framework]),
    source("unused.ts", "invalid syntax {"),
  ]));
  assert.equal(Object.keys(manifest.components).length, 20);
  assert.deepEqual(manifest.components["card-0"]?.hydrationKeys, ["value"]);
  assert.deepEqual(requests.map((request) => request?.openFiles?.length ?? 0), [20, 1, 0]);
  assert.equal(requests[2]?.closeFiles?.length, 21);
  const close = requests[2];
  assert.ok(close && "fileChanges" in close);
  assert.deepEqual(close.fileChanges, { deleted: close.closeFiles });
  assert.equal(clear.mock.callCount(), 1);
});

test("does not create native snapshots for a graph without component candidates", async (t) => {
  const update = t.mock.method(API.prototype, "updateSnapshot");
  const clear = t.mock.method(API.prototype, "clearSourceFileCache");
  const manifest = await compileProjection(context([source("data.ts", "export const value = 1;")]));
  assert.deepEqual(manifest.components, {});
  assert.equal(update.mock.callCount(), 0);
  assert.equal(clear.mock.callCount(), 0);
});

test("releases parsed sources after diagnostics and analyzes fresh bytes on retry", async (t) => {
  const clear = t.mock.method(API.prototype, "clearSourceFileCache");
  const name = "card.ts";
  await assert.rejects(compileProjection(context([
    source(name, "class Broken { define( "),
  ])), /PROJ-C001/);
  assert.equal(clear.mock.callCount(), 1);
  const manifest = await compileProjection(context([source(name, `
import { WebUIElement, observable } from '@microsoft/webui-framework';
class Card extends WebUIElement { @observable current = ''; }
Card.define('fresh-card');
`, [framework])]));
  assert.deepEqual(manifest.components["fresh-card"]?.hydrationKeys, ["current"]);
  assert.equal(clear.mock.callCount(), 2);
});

test("keeps overlapping parser snapshots isolated and releases cache after the last close", (t) => {
  const clear = t.mock.method(API.prototype, "clearSourceFileCache");
  const firstSource = source("shared.ts", "export const value = 1;");
  const secondSource = source("shared.ts", "export const value = 2;");
  const first = createProjectionParser(new Map([[firstSource.id, firstSource]]));
  const second = createProjectionParser(new Map([[secondSource.id, secondSource]]));
  t.after(() => { first.close(); second.close(); });
  first.prepare([firstSource.id]);
  second.prepare([secondSource.id]);
  assert.equal(first.parse(firstSource.id)?.sourceFile.text, firstSource.source);
  assert.equal(second.parse(secondSource.id)?.sourceFile.text, secondSource.source);
  first.close();
  first.close();
  assert.equal(clear.mock.callCount(), 0);
  assert.equal(second.parse(secondSource.id)?.sourceFile.text, secondSource.source);
  second.close();
  assert.equal(clear.mock.callCount(), 1);
});

test("cleans prepared files after a failed snapshot update", (t) => {
  const original = API.prototype.updateSnapshot;
  const requests: Array<Parameters<typeof API.prototype.updateSnapshot>[0]> = [];
  let failed = false;
  t.mock.method(API.prototype, "updateSnapshot", function (
    this: API,
    options: Parameters<typeof API.prototype.updateSnapshot>[0]
  ) {
    requests.push(structuredClone(options));
    if (!failed && options?.openFiles?.length) {
      failed = true;
      throw new Error("snapshot failure");
    }
    return original.call(this, options);
  });
  const clear = t.mock.method(API.prototype, "clearSourceFileCache");
  const module = source("prepared.ts", "export const prepared = true;");
  const parser = createProjectionParser(new Map([[module.id, module]]));
  t.after(() => parser.close());
  assert.throws(() => parser.prepare([module.id]), /snapshot failure/);
  parser.close();
  assert.deepEqual(requests[1]?.closeFiles, requests[0]?.openFiles);
  const close = requests[1];
  assert.ok(close && "fileChanges" in close);
  assert.deepEqual(close.fileChanges, { deleted: requests[0]?.openFiles });
  assert.equal(clear.mock.callCount(), 1);
});
