// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import {
  cp,
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

describe("projection optional peers", () => {
  test("reports PROJ-P001 when TypeScript is absent", async () => {
    const root = await mkdtemp(
      path.join(tmpdir(), "webui-projection-no-typescript-")
    );
    try {
      await cp(
        path.resolve("dist", "projection"),
        path.join(root, "dist", "projection"),
        { recursive: true }
      );
      await writeFile(
        path.join(root, "package.json"),
        JSON.stringify({
          name: "projection-peer-fixture",
          version: "0.0.18",
          type: "module",
        })
      );
      const module = (await import(
        pathToFileURL(
          path.join(root, "dist", "projection", "index.js")
        ).href
      )) as {
        compileProjection(context: unknown): Promise<unknown>;
      };

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
