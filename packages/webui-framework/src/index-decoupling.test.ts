// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * Item 1 (separate streaming entry): the default `@microsoft/webui-framework`
 * entry must NOT pull the streaming coordinator (`streaming.ts`) into its graph.
 * Streaming is opt-in via the `@microsoft/webui-framework/streaming.js` subpath
 * (`streaming-entry.ts`), so a non-streaming host never pays for the coordinator.
 *
 * The build is plain `tsc` (no bundler), so emitted JS preserves every static
 * and dynamic relative import. We walk the real emitted graph from `index.js`
 * and assert the coordinator module is unreachable — a transitive guarantee a
 * single-file source grep cannot give.
 */
describe('default entry decoupling', () => {
  test('dist/index.js has no static or dynamic path to the streaming coordinator', () => {
    // This compiled test lives beside the emitted modules in `dist/`.
    const distDir = dirname(fileURLToPath(import.meta.url));

    const staticImport = /(?:from|import)\s*\(?\s*['"](\.\/[^'"]+)['"]/g;

    const visited = new Set<string>();
    const stack: string[] = ['./index.js'];

    while (stack.length > 0) {
      const spec = stack.pop() as string;
      if (visited.has(spec)) continue;
      visited.add(spec);

      let source: string;
      try {
        source = readFileSync(resolve(distDir, spec), 'utf8');
      } catch {
        // A `.js` specifier with no emitted file (e.g. a types-only import) is
        // not part of the runtime graph — skip it rather than fail the walk.
        continue;
      }

      for (const match of source.matchAll(staticImport)) {
        const next = match[1];
        // Normalize to a `./`-relative specifier keyed off the dist root.
        const abs = resolve(distDir, dirname(spec), next);
        const rel = './' + abs.slice(distDir.length + 1).split('\\').join('/');
        stack.push(rel);
      }
    }

    assert.equal(
      visited.has('./streaming.js'),
      false,
      'the default index entry must not reach the streaming coordinator (streaming.ts)',
    );
    assert.equal(
      visited.has('./streaming-entry.js'),
      false,
      'the default index entry must not reach the streaming entry module either',
    );
    // Sanity: the walk actually traversed a non-trivial graph.
    assert.ok(visited.size > 3, 'import-graph walk should visit multiple modules');
  });
});
