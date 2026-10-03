// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { mkdirSync } from 'node:fs';
import { isAbsolute, join, resolve } from 'node:path';

export function typedIpcBinaryPath(root, targetDir, platform) {
  const output = targetDir
    ? (isAbsolute(targetDir) ? targetDir : resolve(root, targetDir))
    : join(root, 'target');
  mkdirSync(output, { recursive: true });
  return join(output, platform === 'win32'
    ? 'typed-ipc-compile-test.exe'
    : 'typed-ipc-compile-test');
}
