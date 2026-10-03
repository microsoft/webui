// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { mkdtempSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';

import { typedIpcBinaryPath } from './target-output.mjs';

test('creates the target output directory even when Cargo writes elsewhere', () => {
  const root = mkdtempSync(join(tmpdir(), 'webui-typed-ipc-'));
  try {
    const external = join(root, 'external');
    const binary = typedIpcBinaryPath(root, external, 'win32');
    assert.equal(binary, join(external, 'typed-ipc-compile-test.exe'));
    assert.ok(existsSync(dirname(binary)));
    assert.equal(existsSync(join(root, 'target')), false);
  } finally {
    rmSync(root, { recursive: true });
  }
});

test('creates the default target output without a configured target', () => {
  const root = mkdtempSync(join(tmpdir(), 'webui-typed-ipc-'));
  try {
    const binary = typedIpcBinaryPath(root, undefined, 'darwin');
    assert.equal(binary, join(root, 'target', 'typed-ipc-compile-test'));
    assert.ok(existsSync(dirname(binary)));
  } finally {
    rmSync(root, { recursive: true });
  }
});

test('resolves relative Cargo target directories from the workspace root', () => {
  const root = mkdtempSync(join(tmpdir(), 'webui-typed-ipc-'));
  try {
    const binary = typedIpcBinaryPath(root, 'custom-target', 'linux');
    assert.equal(binary, join(root, 'custom-target', 'typed-ipc-compile-test'));
    assert.ok(existsSync(dirname(binary)));
  } finally {
    rmSync(root, { recursive: true });
  }
});
