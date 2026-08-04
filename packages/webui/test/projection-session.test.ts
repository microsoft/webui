// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import {
  mkdtemp,
  readFile,
  readdir,
  rm,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { describe, test } from "node:test";
import {
  ProjectionSession,
  serializeManifestCanonical,
} from "@microsoft/webui/projection.js";
import type {
  AdapterContext,
  ModuleNode,
} from "@microsoft/webui/projection.js";

function context(
  root: string,
  propertyName: string,
  outputContents = "compiled-output"
): AdapterContext {
  const moduleId = path.join(root, "src", "card.ts");
  const outputId = path.join(root, "dist", "index.js");
  const module: ModuleNode = {
    id: moduleId,
    kind: "file",
    source: `
import { observable, WebUIElement } from '@microsoft/webui-framework';
class Card extends WebUIElement { @observable ${propertyName} = ''; }
Card.define('session-card');
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
  };
  return {
    graph: {
      modules: new Map([[moduleId, module]]),
      entries: [moduleId],
    },
    membership: {
      outputs: new Map([[outputId, new Set([moduleId])]]),
    },
    outputContents: new Map([[outputId, outputContents]]),
    rootDir: root,
    manifestPath: path.join(root, "dist", "webui-projection.json"),
    bundlerName: "session-test",
    bundlerVersion: "1.0.0",
  };
}

describe("ProjectionSession", () => {
  test("serializes overlapping finalizations in invocation order", async (t) => {
    const root = await mkdtemp(
      path.join(tmpdir(), "webui-projection-session-")
    );
    t.after(() => rm(root, { recursive: true, force: true }));
    const session = new ProjectionSession();
    const firstContext = context(root, "first");
    const secondContext = context(root, "second");

    const [first, second] = await Promise.all([
      session.finalize(firstContext),
      session.finalize(secondContext),
    ]);

    assert.deepEqual(
      first.components["session-card"]?.hydrationKeys,
      ["first"]
    );
    assert.deepEqual(
      second.components["session-card"]?.hydrationKeys,
      ["second"]
    );
    assert.equal(
      await readFile(secondContext.manifestPath, "utf8"),
      serializeManifestCanonical(second)
    );
    assert.equal(
      (await readdir(path.dirname(secondContext.manifestPath))).some(
        (name) => name.includes(".tmp-")
      ),
      false
    );
  });

  test("preserves the previous manifest and recovers after failure", async (t) => {
    const root = await mkdtemp(
      path.join(tmpdir(), "webui-projection-session-recovery-")
    );
    t.after(() => rm(root, { recursive: true, force: true }));
    const session = new ProjectionSession();
    const initialContext = context(root, "initial");
    await session.finalize(initialContext);
    const before = await readFile(initialContext.manifestPath, "utf8");
    const invalidContext = {
      ...context(root, "invalid"),
      outputContents: new Map(),
    };

    await assert.rejects(
      session.finalize(invalidContext),
      /PROJ-C014/
    );
    assert.equal(
      await readFile(initialContext.manifestPath, "utf8"),
      before
    );

    const recovered = await session.finalize(context(root, "recovered"));
    assert.deepEqual(
      recovered.components["session-card"]?.hydrationKeys,
      ["recovered"]
    );
  });
});
