// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { execFileSync, spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

// Use Cargo's exact artifacts rather than selecting a possibly stale glob.
const root = fileURLToPath(new URL('../../../', import.meta.url));
const result = spawnSync('cargo', [
  'build', '-p', 'microsoft-webui-desktop', '--no-default-features',
  '--features', 'application-ipc', '--lib', '--message-format=json',
], { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });
if (result.error) throw result.error;
const libraries = new Map();
for (const line of result.stdout.split('\n')) {
  if (!line) continue;
  const artifact = JSON.parse(line);
  if (artifact.reason === 'compiler-message' && artifact.message.rendered) {
    process.stderr.write(artifact.message.rendered);
  }
  if (artifact.reason !== 'compiler-artifact') continue;
  const library = artifact.filenames.find(name => name.endsWith('.rlib'));
  if (library) libraries.set(artifact.target.name, library);
}
if (result.status !== 0) process.exit(result.status ?? 1);
const sdk = libraries.get('webui_desktop');
const prost = libraries.get('prost');
if (!sdk || !prost) throw new Error('Cargo did not emit the SDK and prost library artifacts');
const binary = join(root, 'target', process.platform === 'win32' ? 'typed-ipc-compile-test.exe' : 'typed-ipc-compile-test');
execFileSync('rustc', [
  '--test', '--edition=2021', fileURLToPath(new URL('support/runtime.rs', import.meta.url)),
  '--extern', `webui_desktop=${sdk}`, '--extern', `prost=${prost}`,
  '-L', `dependency=${dirname(prost)}`, '-o', binary,
], { cwd: root, stdio: 'inherit' });
const tests = execFileSync(binary, ['--list', '--format=terse'], { cwd: root, encoding: 'utf8' });
if (!tests.split('\n').some(line => line.endsWith(': test'))) {
  throw new Error('Generated Rust compile check contains no tests');
}
execFileSync(binary, [], { cwd: root, stdio: 'inherit' });
