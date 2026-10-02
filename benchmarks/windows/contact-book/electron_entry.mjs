// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { pathToFileURL } from 'node:url';

const appDist = process.env.WEBUI_BENCHMARK_APP_DIST;
const statePath = process.env.WEBUI_BENCHMARK_STATE_PATH;
const mainPath = process.env.WEBUI_BENCHMARK_ELECTRON_MAIN;
if (!appDist || !statePath || !mainPath) {
  throw new Error('benchmark Electron paths are not configured');
}

// Electron consumes its own switches before the entry script. Normalize the
// argv seen by the existing launcher without changing that launcher or bundle.
process.argv = [process.argv[0], process.argv[1], appDist, statePath, '--plugin=webui'];
await import(pathToFileURL(mainPath).href);
