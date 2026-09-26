// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const root = fileURLToPath(new URL('../', import.meta.url));
const platforms: Record<string, string> = {
  'darwin-arm64': '@microsoft/webui-desktop-darwin-arm64',
  'darwin-x64': '@microsoft/webui-desktop-darwin-x64',
  'linux-arm64': '@microsoft/webui-desktop-linux-arm64',
  'linux-x64': '@microsoft/webui-desktop-linux-x64',
  'win32-arm64': '@microsoft/webui-desktop-win32-arm64',
  'win32-x64': '@microsoft/webui-desktop-win32-x64',
};

function packageVersion(manifest: string): string {
  const parsed: unknown = JSON.parse(readFileSync(manifest, 'utf8'));
  if (!parsed || typeof parsed !== 'object' || !('version' in parsed) || typeof parsed.version !== 'string') {
    throw new Error(`Invalid package version in ${manifest}`);
  }
  return parsed.version;
}

function nativeBinary(): string {
  const key = `${process.platform}-${process.arch}`;
  const packageName = platforms[key];
  if (!packageName) {
    throw new Error(`Unsupported desktop platform ${key}. Supported: ${Object.keys(platforms).join(', ')}`);
  }

  let manifest: string;
  try {
    manifest = require.resolve(`${packageName}/package.json`);
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'MODULE_NOT_FOUND') {
      throw new Error(
        `Missing ${packageName}. Reinstall @microsoft/webui-desktop with optional dependencies enabled.`,
        { cause: error },
      );
    }
    throw error;
  }

  const expectedVersion = packageVersion(path.join(root, 'package.json'));
  const installedVersion = packageVersion(manifest);
  if (installedVersion !== expectedVersion) {
    throw new Error(
      `${packageName} is version ${installedVersion}, but @microsoft/webui-desktop is ${expectedVersion}. Reinstall matching desktop packages.`,
    );
  }

  const binary = path.join(path.dirname(manifest), 'bin', process.platform === 'win32' ? 'webui-desktop.exe' : 'webui-desktop');
  if (!existsSync(binary)) {
    throw new Error(`Missing desktop binary at ${binary}. Reinstall ${packageName}.`);
  }
  return binary;
}

try {
  const result = spawnSync(nativeBinary(), process.argv.slice(2), { stdio: 'inherit' });
  if (result.error) {
    throw result.error;
  }
  if (result.signal) {
    process.kill(process.pid, result.signal);
  } else {
    process.exitCode = result.status ?? 1;
  }
} catch (error) {
  console.error(`[webui-desktop] ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
}
