// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import * as path from "node:path";

import {
  ProjectionError,
  createDiagnostic,
} from "./diagnostics.js";

/**
 * Computes the build root that contains every physical projection artifact.
 *
 * `AdapterContext.rootDir` anchors every manifest key: inputs, outputs, and
 * component modules are all stored as paths relative to it. Adapters therefore
 * need the deepest directory containing the manifest and every physical input
 * and output, which is what this returns.
 *
 * Paths that live on different filesystem roots have no such directory. On
 * Windows that is a real configuration, not a corner case: a `TEMP` directory
 * on `C:` while the project sits on another drive puts generated inputs on a
 * different volume than the outputs. This reports that as `PROJ-C015` naming
 * both paths, because the only fix is to move one of them.
 *
 * @param paths Absolute (or resolvable) paths to the manifest and to every
 *   physical input and output. Must contain at least one entry.
 * @returns The absolute directory containing all of them.
 * @throws {ProjectionError} `PROJ-C015` when the paths span filesystem roots.
 */
export function resolveBuildRoot(
  paths: ReadonlyArray<string>
): string {
  const first = paths[0];
  if (first === undefined) {
    throw new ProjectionError([
      createDiagnostic("PROJ-C015", {
        help: "Provide the manifest path and at least one physical input or output so a build root can be derived.",
      }),
    ]);
  }
  let root = path.dirname(path.resolve(first));
  for (let index = 1; index < paths.length; index++) {
    const directory = path.dirname(path.resolve(paths[index]!));
    while (!isWithin(root, directory)) {
      const parent = path.dirname(root);
      if (parent === root) {
        throw crossRootError(first, paths[index]!);
      }
      root = parent;
    }
  }
  return root;
}

/**
 * Reports whether `candidate` is `root` or lives below it.
 */
export function isWithin(root: string, candidate: string): boolean {
  const relative = path.relative(root, candidate);
  return (
    relative.length === 0 ||
    (relative !== ".." &&
      !relative.startsWith(`..${path.sep}`) &&
      !path.isAbsolute(relative))
  );
}

/**
 * Builds the `PROJ-C015` diagnostic for two artifacts on different roots.
 *
 * Error construction is a cold path, so the message is assembled here rather
 * than inlined into the scanning loop above.
 */
function crossRootError(
  first: string,
  second: string
): ProjectionError {
  const left = path.resolve(first);
  const right = path.resolve(second);
  return new ProjectionError([
    createDiagnostic("PROJ-C015", {
      location: right,
      help:
        `"${left}" and "${right}" are on different filesystem roots, so no single build root contains both. ` +
        "Keep one bundler invocation on a single filesystem volume: on Windows, point TEMP/TMP at a directory " +
        "on the drive that holds the project, or move the project onto the drive that holds TEMP.",
    }),
  ]);
}
