// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  mkdir,
  open,
  rename,
  rm,
} from "node:fs/promises";
import * as path from "node:path";
import type { AdapterContext } from "./graph.js";
import type { ProjectionManifest } from "./manifest.js";
import { serializeManifestCanonical } from "./manifest.js";
import {
  compileProjection,
  preloadProjectionCompiler,
} from "./loader.js";

let temporaryFileSequence = 0;

/**
 * Finalizes complete bundler adapter contexts into projection manifests.
 *
 * A session serializes overlapping finalizations in invocation order.
 * Adapters must invoke `finalize()` in compilation order; bundler-specific graph
 * collection, path derivation, version checks, and diagnostic presentation
 * remain adapter responsibilities.
 */
export class ProjectionSession {
  readonly #compilerLoad = preloadProjectionCompiler().then(
    () => ({ loaded: true as const }),
    (error: unknown) => ({ loaded: false as const, error })
  );
  #pending: Promise<void> = Promise.resolve();

  /**
   * Compile and atomically replace the manifest declared by `context`.
   *
   * The previous manifest remains intact when compilation, serialization, or
   * the temporary-file write fails.
   */
  finalize(context: AdapterContext): Promise<ProjectionManifest> {
    const completion = this.#pending.then(async () => {
      const compiler = await this.#compilerLoad;
      if (!compiler.loaded) throw compiler.error;
      const manifest = await compileProjection(context);
      await writeAtomic(
        context.manifestPath,
        serializeManifestCanonical(manifest)
      );
      return manifest;
    });
    this.#pending = completion.then(
      () => undefined,
      () => undefined
    );
    return completion;
  }
}

async function writeAtomic(
  manifestPath: string,
  contents: string
): Promise<void> {
  await mkdir(path.dirname(manifestPath), { recursive: true });
  const sequence = temporaryFileSequence++;
  const temporaryPath = `${manifestPath}.tmp-${process.pid}-${sequence}`;
  try {
    const handle = await open(temporaryPath, "wx");
    try {
      await handle.writeFile(contents, "utf8");
      await handle.sync();
    } finally {
      await handle.close();
    }
    await rename(temporaryPath, manifestPath);
  } finally {
    await rm(temporaryPath, { force: true });
  }
}
