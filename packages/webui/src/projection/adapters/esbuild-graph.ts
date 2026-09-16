// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { Metafile } from "esbuild";

/** Other output keys reachable from an entry, ordered largest first. */
export function outputImportClosure(
  metafile: Metafile,
  entry: string,
  includeDynamic = false
): string[] {
  const reached = new Set([entry]);
  const pending = [entry];
  const members: string[] = [];
  while (pending.length > 0) {
    const current = pending.pop()!;
    for (const edge of metafile.outputs[current]?.imports ?? []) {
      if ((!includeDynamic && edge.kind !== "import-statement") || edge.external) continue;
      if (reached.has(edge.path)) continue;
      reached.add(edge.path);
      pending.push(edge.path);
      members.push(edge.path);
    }
  }
  return members.sort((left, right) => {
    const bySize = (metafile.outputs[right]?.bytes ?? 0) - (metafile.outputs[left]?.bytes ?? 0);
    return bySize || Buffer.compare(Buffer.from(left), Buffer.from(right));
  });
}
