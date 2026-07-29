// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Builds this example as two entry points instead of the shared default's one,
 * and records which chunks the critical entry needs so the server can preload
 * them.
 *
 * `index.js` is the critical entry: the streaming coordinator plus the
 * components the composer and feed boundaries hydrate. It loads `async` from
 * `<head>` because those boundaries must become interactive as early as
 * possible.
 *
 * `weather-panel.js` is an island. Its `<script>` lives *inside* the weather
 * boundary, so the browser only discovers it when that chunk reaches the
 * parser and the critical entry never carries its bytes. Code splitting hoists
 * the framework runtime into a chunk both entries share, so the island costs
 * only its own component code rather than a duplicated runtime.
 *
 * Splitting has one hazard worth naming: the shared chunk is a static import
 * of `index.js`, so without a hint the browser cannot discover it until
 * `index.js` has downloaded and parsed. That waterfall costs a round trip and
 * measurably erases the benefit. The fix is a `modulepreload` for each chunk
 * the critical entry needs — but esbuild content-hashes those filenames, so
 * they cannot be hard-coded in `index.html`. This build writes them to
 * `dist/critical-modules.json`; `server/src/preload.rs` turns that into the
 * `<link rel="modulepreload">` tags bound to `{{{modulePreloads}}}`.
 *
 * The island is deliberately excluded: preloading it would put the code back
 * on the critical path that moving it out of `index.js` just removed.
 */
import { writeFileSync } from "node:fs";
import path from "node:path";
import { runWebUIClientBuild } from "../../build-client.mjs";

const CRITICAL_ENTRY = "src/index.ts";
const MANIFEST = "dist/critical-modules.json";

/**
 * Transitive closure of an output's static imports, as browser paths, ordered
 * largest first.
 *
 * Order is not cosmetic. The browser issues preloads in document order and
 * they share the connection, so listing a 284-byte chunk ahead of a 35 KiB one
 * delays the long pole behind it — measured at 1076 ms versus 951 ms composer
 * time-to-interactive on this example, a larger effect than splitting the
 * island in the first place.
 *
 * Dynamic imports are skipped on purpose: they are demand-loaded (the
 * framework's hydration-mismatch reporter, for one), so preloading them would
 * download code the page may never run.
 */
function criticalChunks(metafile, entryPoint) {
  const outputs = metafile.outputs;
  const entry = Object.keys(outputs).find(
    (file) => outputs[file].entryPoint === entryPoint,
  );
  if (!entry) {
    throw new Error(`No build output has entry point ${entryPoint}`);
  }

  // Iterative worklist rather than recursion — chunk graphs can be deep and
  // may contain cycles.
  const seen = new Set([entry]);
  const chunks = [];
  const pending = [entry];
  while (pending.length > 0) {
    const current = pending.pop();
    for (const imported of outputs[current]?.imports ?? []) {
      if (imported.kind !== "import-statement" || imported.external) continue;
      if (seen.has(imported.path)) continue;
      seen.add(imported.path);
      pending.push(imported.path);
      chunks.push({
        href: `./${path.relative("dist", imported.path).replace(/\\/g, "/")}`,
        bytes: outputs[imported.path]?.bytes ?? 0,
      });
    }
  }

  // Path breaks ties so the manifest is byte-stable across rebuilds.
  chunks.sort((a, b) => b.bytes - a.bytes || a.href.localeCompare(b.href));
  return chunks.map((chunk) => chunk.href);
}

await runWebUIClientBuild({
  // Object form so the island lands at `dist/weather-panel.js` rather than
  // inheriting its `src/weather-panel/` directory.
  entryPoints: {
    index: CRITICAL_ENTRY,
    "weather-panel": "src/weather-panel/weather-panel.ts",
  },
  // A plugin rather than a post-build step so `--watch` rewrites the
  // manifest on every rebuild; chunk hashes change whenever their contents
  // do, and a stale hint would preload a file that no longer exists.
  plugins: [
    {
      name: "webui-critical-modules",
      setup(build) {
        build.onEnd((result) => {
          if (!result.metafile) return;
          writeFileSync(
            MANIFEST,
            `${JSON.stringify(criticalChunks(result.metafile, CRITICAL_ENTRY), null, 2)}\n`,
          );
        });
      },
    },
  ],
});
