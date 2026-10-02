// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { readFileSync } from 'node:fs';
import {
  brotliCompressSync,
  constants,
  gzipSync,
} from 'node:zlib';

// Release production build at base e5fc217e6c88e356446330c4c3091196a0ce7b22.
const BASELINE = {
  startupRaw: 90_625,
  startupGzip: 27_874,
  startupBrotli: 24_616,
  totalRaw: 95_336,
  totalGzip: 30_034,
  totalBrotli: 26_465,
  coldRaw: 1_749,
  coldGzip: 818,
  coldBrotli: 694,
  startupRequests: 2,
  coldRequests: 2,
};

const BUDGETS = {
  startupRaw: BASELINE.startupRaw,
  startupGzip: BASELINE.startupGzip,
  startupBrotli: BASELINE.startupBrotli,
  totalRaw: BASELINE.totalRaw,
  totalGzip: BASELINE.totalGzip,
  totalBrotli: BASELINE.totalBrotli,
  coldRaw: Math.ceil(BASELINE.coldRaw * 1.1),
  coldGzip: Math.ceil(BASELINE.coldGzip * 1.1),
  coldBrotli: Math.ceil(BASELINE.coldBrotli * 1.1),
  startupRequests: 3,
  coldRequests: BASELINE.coldRequests,
};

const metafile = JSON.parse(readFileSync('dist/client-metafile.json', 'utf8'));
const outputs = metafile.outputs;
const entry = findOutput('src/index.ts');
const lazy = findOutput('.webui/lazy-panel.webui.js');
const startup = staticClosure(entry);
const cold = staticClosure(lazy);
for (const output of startup) cold.delete(output);

const allJavaScript = new Set(
  Object.keys(outputs).filter(path => path.endsWith('.js')),
);
const measurements = {
  ...sizes('startup', startup),
  ...sizes('total', allJavaScript),
  ...sizes('cold', cold),
  startupRequests: startup.size,
  coldRequests: cold.size,
};

let failed = false;
console.log('| Metric | Main | Current | Delta | Budget |');
console.log('|---|---:|---:|---:|---:|');
for (const key of Object.keys(BUDGETS)) {
  const baseline = BASELINE[key];
  const current = measurements[key];
  const delta = ((current - baseline) / baseline) * 100;
  const budget = BUDGETS[key];
  if (current > budget) failed = true;
  console.log(
    `| ${key} | ${baseline} | ${current} | ${delta >= 0 ? '+' : ''}${delta.toFixed(1)}% | <= ${budget} |`,
  );
}

if (failed) {
  throw new Error('Component asset production bundle exceeded its performance budget.');
}

function findOutput(entryPoint) {
  const match = Object.entries(outputs)
    .find(([, output]) => output.entryPoint === entryPoint);
  if (!match) throw new Error(`Missing bundle output for ${entryPoint}.`);
  return match[0];
}

function staticClosure(entryPoint) {
  const closure = new Set();
  const pending = [entryPoint];
  while (pending.length > 0) {
    const path = pending.pop();
    if (closure.has(path)) continue;
    closure.add(path);
    const output = outputs[path];
    if (!output) throw new Error(`Missing bundle output ${path}.`);
    for (const imported of output.imports ?? []) {
      if (imported.kind === 'import-statement' && !imported.external) {
        pending.push(imported.path);
      }
    }
  }
  return closure;
}

function sizes(prefix, paths) {
  let raw = 0;
  let gzip = 0;
  let brotli = 0;
  for (const path of paths) {
    const bytes = readFileSync(path);
    raw += bytes.length;
    gzip += gzipSync(bytes, { level: 9 }).length;
    brotli += brotliCompressSync(bytes, {
      params: { [constants.BROTLI_PARAM_QUALITY]: 11 },
    }).length;
  }
  return {
    [`${prefix}Raw`]: raw,
    [`${prefix}Gzip`]: gzip,
    [`${prefix}Brotli`]: brotli,
  };
}
