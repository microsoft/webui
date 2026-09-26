// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const workspace = fileURLToPath(new URL('../../../', import.meta.url));
const packageName = process.argv[2];
assert.match(packageName, /^webui-desktop-(?:darwin|linux|win32)-(?:x64|arm64)$/);
const root = path.join(workspace, 'packages', packageName);
const manifest = JSON.parse(readFileSync(path.join(root, 'package.json'), 'utf8'));
const binName = packageName.includes('-win32-') ? 'webui-desktop.exe' : 'webui-desktop';
const binary = path.join(root, 'bin', binName);
assert(statSync(binary).size > 0, `missing staged ${packageName} binary`);
const exported = process.env.WEBUI_NATIVE_EXPORT_ROOT;
if (exported) {
  const exportBinary = path.join(exported, 'packages', packageName, 'bin', binName);
  assert.equal(statSync(exportBinary).size, statSync(binary).size, `missing exported ${packageName} binary`);
}

const output = mkdtempSync(path.join(tmpdir(), 'webui-desktop-package-'));
try {
  const packed = spawnSync('pnpm', ['--dir', root, 'pack', '--pack-destination', output], {
    cwd: workspace,
    encoding: 'utf8',
    shell: process.platform === 'win32',
  });
  assert.equal(packed.status, 0, `${packed.stdout}\n${packed.stderr}`);
  const tarball = path.join(output, `microsoft-${packageName}-${manifest.version}.tgz`);
  const listed = spawnSync('tar', ['-tvzf', tarball], { encoding: 'utf8' });
  assert.equal(listed.status, 0, `${listed.stdout}\n${listed.stderr}`);
  const lines = listed.stdout.split('\n').map(line => line.trimEnd());
  const binaryLine = lines.find(line => line.endsWith(`package/bin/${binName}`));
  assert(binaryLine, `packed ${packageName} is missing ${binName}:\n${listed.stdout}`);
  if (binName === 'webui-desktop') {
    assert.match(binaryLine, /^-rwx/, `packed ${binName} is not executable`);
  } else {
    for (const name of [
      'Microsoft.WindowsAppRuntime.Bootstrap.dll',
      'Microsoft.WindowsAppSDK.LICENSE.txt',
      'Microsoft.WindowsAppSDK.NOTICES.txt',
      'Microsoft.WindowsAppSDK.PROVENANCE.json',
    ]) {
      assert(lines.some(line => line.endsWith(`package/bin/${name}`)), `packed ${packageName} is missing ${name}`);
      if (exported) {
        assert(statSync(path.join(exported, 'packages', packageName, 'bin', name)).size > 0,
          `missing exported ${packageName} companion ${name}`);
      }
    }
  }
} finally {
  rmSync(output, { recursive: true, force: true });
}
