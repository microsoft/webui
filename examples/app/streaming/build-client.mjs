// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Builds this example as two entry points instead of the shared default's one.
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
 * Splitting has one hazard worth naming, because it is the reason this pattern
 * is not yet a recommendation: the shared chunk is a static import of
 * `index.js`, so the preload scanner cannot discover it until `index.js` has
 * downloaded and parsed. That waterfall costs a round trip. A
 * `<link rel="modulepreload">` for each shared chunk removes it, but esbuild
 * content-hashes those filenames, so today an application would have to grow
 * its own build manifest and template it into `<head>` itself. WebUI should
 * emit those hints from the boundary's component closure instead — see the
 * README for the measurements that motivate it.
 */
import { runWebUIClientBuild } from "../../build-client.mjs";

await runWebUIClientBuild({
  // Object form so the island lands at `dist/weather-panel.js` rather than
  // inheriting its `src/weather-panel/` directory.
  entryPoints: {
    index: "src/index.ts",
    "weather-panel": "src/weather-panel/weather-panel.ts",
  },
});