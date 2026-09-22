// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import fs from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {
  binaryNameFor,
  packageNameFor,
  platformKey,
  resolveBinaryFrom,
} from '../platform.mjs';

test('webui-press package maps supported platforms to native packages', () => {
  assert.equal(platformKey('linux', 'x64'), 'linux-x64');
  assert.equal(packageNameFor('linux', 'x64'), '@microsoft/webui-press-linux-x64');
  assert.equal(packageNameFor('darwin', 'arm64'), '@microsoft/webui-press-darwin-arm64');
  assert.equal(packageNameFor('win32', 'arm64'), '@microsoft/webui-press-win32-arm64');
  assert.equal(binaryNameFor('linux'), 'webui-press');
  assert.equal(binaryNameFor('win32'), 'webui-press.exe');
});

test('webui-press package resolves overrides, platform packages, and local builds', (t) => {
  const root = fs.mkdtempSync(path.join(tmpdir(), 'webui-press-package-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  const override = path.join(root, 'custom-webui-press');
  assert.equal(resolveBinaryFrom({ env: { WEBUI_PRESS_BINARY_PATH: override } }), override);

  const platformPackage = path.join(root, 'platform-package');
  const packageBin = path.join(platformPackage, 'bin', 'webui-press');
  fs.mkdirSync(path.dirname(packageBin), { recursive: true });
  fs.writeFileSync(packageBin, '');
  assert.equal(
    resolveBinaryFrom({ env: {}, platform: 'linux', arch: 'x64', packageBase: platformPackage }),
    packageBin,
  );

  const localBin = path.join(root, 'target', 'debug', 'webui-press.exe');
  fs.mkdirSync(path.dirname(localBin), { recursive: true });
  fs.writeFileSync(localBin, '');
  assert.equal(
    resolveBinaryFrom({ env: {}, platform: 'win32', arch: 'x64', workspaceRoot: root }),
    localBin,
  );
});
