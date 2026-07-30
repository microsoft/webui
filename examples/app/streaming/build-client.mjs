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
 * Splitting has one hazard worth naming: the shared chunk is a static import
 * of `index.js`, so the preload scanner cannot discover it until `index.js`
 * has downloaded and parsed. That waterfall costs a round trip and cancels
 * out what splitting saves. WebUI closes it automatically — the projection
 * manifest this build writes records each entry's static import closure in
 * descending output size, and the WebUI build turns that into size-ordered
 * `<link rel="modulepreload">` hints. Nothing here has to opt in, and the
 * island is excluded on its own because its loader lives inside a boundary.
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