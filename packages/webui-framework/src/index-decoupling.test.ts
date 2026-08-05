// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * Item 1 (separate streaming entry): the default `@microsoft/webui-framework`
 * entry must NOT pull the streaming coordinator into its graph. Streaming is
 * opt-in via the `@microsoft/webui-framework/streaming.js` subpath
 * (`streaming-entry.ts`), so a non-streaming host never pays for the
 * coordinator.
 *
 * Same guarantee applies to visible hydration: the default entry must not
 * pull in the viewport/interaction coordinator. That is opt-in via the
 * `@microsoft/webui-framework/visible-hydration.js` subpath
 * (`visible-hydration-entry.ts`); `static hydration = 'visible'` alone falls
 * back to eager without it.
 *
 * The build is plain `tsc` (no bundler), so emitted JS preserves every static
 * and dynamic relative import. We walk the real emitted graph from `index.js`
 * and assert every coordinator module is unreachable — a transitive guarantee
 * a single-file source grep cannot give.
 *
 * The forbidden set is discovered from the emitted files rather than hard-coded,
 * so splitting a coordinator into more modules cannot silently escape it.
 */

/**
 * The one streaming module the default entry is allowed to reach.
 *
 * `streaming-mode.js` is a dependency-free leaf holding mode detection and the
 * shared hook symbols. `template-element.ts` must consult
 * `isStreamingHydrationMode()` on the hot hydration path, and importing the
 * coordinator instead would close a cycle
 * (`streaming.ts` → `static-host.ts` → `template-element.ts`). It contains no
 * coordinator state and pulls nothing else in.
 */
const ALLOWED_STREAMING_MODULES = new Set(['./streaming-mode.js']);

/**
 * The one visible-hydration module the default entry is allowed to reach.
 *
 * `lazy-hydration.js` is a dependency-free contract: the activation symbol,
 * types, and a coordinator registry that `element.ts` consults through an
 * optional-chained reference. It contains no `IntersectionObserver` usage,
 * activation queue, or DOM traversal — that all lives in
 * `visible-hydration-coordinator.js`, reachable only from the optional
 * `@microsoft/webui-framework/visible-hydration.js` entry.
 */
const ALLOWED_VISIBLE_HYDRATION_MODULES = new Set(['./lazy-hydration.js']);

/**
 * Walk the emitted static/dynamic import graph starting from `./index.js`.
 * The build is plain `tsc` (no bundler), so emitted JS preserves every
 * relative import verbatim — a transitive guarantee a single-file source grep
 * cannot give.
 */
function walkIndexImportGraph(distDir: string): Set<string> {
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

  return visited;
}

describe('default entry decoupling', () => {
  test('dist/index.js has no static or dynamic path to any streaming coordinator module', () => {
    // This compiled test lives beside the emitted modules in `dist/`.
    const distDir = dirname(fileURLToPath(import.meta.url));

    const streamingModules = readdirSync(distDir)
      .filter(
        (name) =>
          name.startsWith('streaming') &&
          name.endsWith('.js') &&
          !name.endsWith('.test.js'),
      )
      .map((name) => `./${name}`);

    assert.ok(
      streamingModules.length > 1,
      'expected the emitted streaming modules to be discoverable in dist/',
    );
    for (const allowed of ALLOWED_STREAMING_MODULES) {
      assert.ok(
        streamingModules.includes(allowed),
        `${allowed} is allow-listed but was not emitted — update the allow-list`,
      );
    }

    const visited = walkIndexImportGraph(distDir);

    const leaked = streamingModules.filter(
      (name) => visited.has(name) && !ALLOWED_STREAMING_MODULES.has(name),
    );
    assert.deepEqual(
      leaked,
      [],
      `the default index entry must not reach any streaming coordinator module, but reached: ${leaked.join(', ')}`,
    );
    // Sanity: the walk actually traversed a non-trivial graph.
    assert.ok(visited.size > 3, 'import-graph walk should visit multiple modules');
  });

  test('dist/index.js has no static or dynamic path to the visible-hydration coordinator', () => {
    const distDir = dirname(fileURLToPath(import.meta.url));

    const visibleHydrationModules = readdirSync(distDir)
      .filter(
        (name) =>
          (name.startsWith('lazy-hydration') || name.startsWith('visible-hydration')) &&
          name.endsWith('.js') &&
          !name.endsWith('.test.js'),
      )
      .map((name) => `./${name}`);

    assert.ok(
      visibleHydrationModules.length > 1,
      'expected the emitted visible-hydration modules to be discoverable in dist/',
    );
    for (const allowed of ALLOWED_VISIBLE_HYDRATION_MODULES) {
      assert.ok(
        visibleHydrationModules.includes(allowed),
        `${allowed} is allow-listed but was not emitted — update the allow-list`,
      );
    }

    const visited = walkIndexImportGraph(distDir);

    const leaked = visibleHydrationModules.filter(
      (name) => visited.has(name) && !ALLOWED_VISIBLE_HYDRATION_MODULES.has(name),
    );
    assert.deepEqual(
      leaked,
      [],
      `the default index entry must not reach the visible-hydration coordinator, but reached: ${leaked.join(', ')}`,
    );
  });
});
