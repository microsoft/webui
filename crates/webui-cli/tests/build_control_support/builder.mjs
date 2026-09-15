// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { appendFile, readFile, rm, writeFile } from 'node:fs/promises';
import { isAbsolute, join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

export default async function createBuilder({ appDir, outDir }) {
  if (!isAbsolute(appDir) || !isAbsolute(outDir)) {
    throw new Error('Builder paths must be absolute');
  }
  const readInput = async () => JSON.parse(await readFile(join(appDir, 'input.json'), 'utf8'));
  const record = async (kind, input, details = {}) => appendFile(
    join(outDir, 'calls.ndjson'),
    JSON.stringify({ kind, title: input.title, pid: process.pid, ...details }) + '\n',
  );
  const initial = await readInput();
  await writeFile(join(outDir, 'worker.pid'), String(process.pid));
  await record('factory', initial, { appDir, outDir });
  await delay(initial.initDelayMs ?? 0);
  if (initial.initFailure) throw new Error('Fixture factory failed');
  if (initial.factoryCrash) process.exit(31);
  await record('initialized', initial);
  console.log('fixture factory stdout is not a protocol');
  console.error('fixture factory stderr');
  let active = 0;
  let last = initial;
  return {
    watchPaths: ['input.json', '../extra-input.json', '../shared-inputs'],
    async rebuild() {
      const input = await readInput();
      last = input;
      active += 1;
      await record('begin', input, { active });
      try {
        await delay(input.delayMs);
        if (input.crash) process.exit(32);
        if (input.mode === 'missing') {
          await rm(join(outDir, 'theme.json'), { force: true });
          await rm(join(outDir, 'state.json'), { force: true });
        } else {
          const themes = { light: { brand: input.brand } };
          if (input.dark) themes.dark = { brand: '#abcdef' };
          let theme = JSON.stringify({ themes });
          if (input.mode === 'bad-theme') theme = '{';
          if (input.mode === 'missing-token') theme = '{"themes":{"light":{"unrelated":"red"}}}';
          const state = input.mode === 'bad-state' ? '{' : JSON.stringify({
            title: input.title, visible: true, appValue: 'preserved',
            tokens: { light: 'must-be-replaced', custom: 'unrelated-token-value' },
          });
          await writeFile(join(outDir, 'theme.json'), theme);
          await writeFile(join(outDir, 'state.json'), state);
        }
        await writeFile(
          join(outDir, 'client.js'),
          `globalThis.clientTitle = ${JSON.stringify(input.title)};`,
        );
        await record('written', input);
        if (input.fail) throw new Error('Client plugin failed after emitting files: café');
        if (input.idleCrashMs) setTimeout(() => process.exit(33), input.idleCrashMs);
      } finally {
        active -= 1;
        await record('end', input, { active });
      }
    },
    async dispose() {
      await record('dispose-begin', last);
      if (last.disposeHang) await new Promise(() => {});
      await delay(last.disposeDelayMs ?? 0);
      if (last.disposeFailure) throw new Error('Fixture disposal failed');
      await record('dispose-end', last);
    },
  };
}
