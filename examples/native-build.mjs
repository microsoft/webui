// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { spawnSync } from 'node:child_process';
import { statSync } from 'node:fs';
import path from 'node:path';

function preparedPath(name) {
  const value = process.env[name];
  if (!value) return undefined;
  if (!path.isAbsolute(value) || !statSync(value, { throwIfNoEntry: false })?.isFile()) {
    throw new Error(`${name} must point to an existing absolute file: ${value}`);
  }
  return value;
}

function commandFor(mode, args) {
  if (mode === 'cli') {
    const binary = preparedPath('WEBUI_BINARY_PATH');
    return binary ? [binary, args] : ['cargo', ['run', '-p', 'microsoft-webui-cli', '--', ...args]];
  }
  if ((mode !== 'addons' && mode !== 'client') || (mode === 'client' && args.length === 0)
    || (mode === 'addons' && args.length !== 0)) {
    throw new Error('Usage: node native-build.mjs addons | client <script> [args...] | cli [args...]');
  }
  const runtime = preparedPath('WEBUI_ADDON_PATH');
  const builder = preparedPath('WEBUI_BUILD_ADDON_PATH');
  if (Boolean(runtime) !== Boolean(builder)) {
    throw new Error('Set both WEBUI_ADDON_PATH and WEBUI_BUILD_ADDON_PATH, or neither for a standalone build.');
  }
  if (runtime && builder) {
    return mode === 'client' ? [process.execPath, args] : undefined;
  }
  const cargoArgs = ['xtask', 'build-node-addons', '--release'];
  if (mode === 'client') cargoArgs.push('--', process.execPath, ...args);
  return ['cargo', cargoArgs];
}

try {
  // The parallel parent owns native preparation; standalone builds still use Cargo.
  const command = commandFor(process.argv[2], process.argv.slice(3));
  if (command) {
    const [program, args] = command;
    const result = spawnSync(program, args, { stdio: 'inherit' });
    if (result.error) throw result.error;
    if (result.signal) throw new Error(`${program} terminated by ${result.signal}`);
    process.exitCode = result.status ?? 1;
  }
} catch (error) {
  console.error(error);
  process.exitCode = 1;
}
