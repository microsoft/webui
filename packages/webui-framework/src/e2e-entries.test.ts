// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { dirname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import { API } from 'typescript/unstable/sync';

test('manual and routed E2E application sources type-check without generated lazy artifacts', () => {
  const configPath = fileURLToPath(new URL('../tsconfig.test.json', import.meta.url));
  const root = dirname(configPath);
  const entries = [
    resolve(root, 'tests/fixtures/lazy-pages/application.ts'),
    resolve(root, '../webui-delivery/tests/fixtures/lazy-routes/index.ts'),
  ];
  const canonical = (path: string): string =>
    process.platform === 'win32' ? resolve(path).toLowerCase() : resolve(path);
  // Warm worktree output must not make this fresh-checkout regression pass.
  const generated = canonical(resolve(root, 'tests/fixtures/dist/lazy-pages/generated'));
  const isGenerated = (path: string): boolean => {
    const absolute = canonical(path);
    return absolute === generated || absolute.startsWith(generated + sep);
  };
  const api = new API({
    cwd: root,
    fs: {
      fileExists: path => isGenerated(path) ? false : undefined,
      directoryExists: path => isGenerated(path) ? false : undefined,
      readFile: path => isGenerated(path) ? null : undefined,
    },
  });
  try {
    const snapshot = api.updateSnapshot({ openProjects: [configPath] });
    const project = snapshot.getProject(configPath);
    assert.ok(project);
    const program = project.program;
    for (const entry of entries) {
      assert.ok(program.getSourceFileNames().some(source => canonical(source) === canonical(entry)));
    }
    const diagnostics = [
      ...program.getConfigFileParsingDiagnostics(), ...program.getGlobalDiagnostics(),
      ...program.getProgramDiagnostics(), ...program.getSyntacticDiagnostics(),
      ...program.getSemanticDiagnostics(),
    ];
    assert.deepEqual(diagnostics.map(diagnostic =>
      `${diagnostic.fileName ?? configPath}: TS${diagnostic.code}: ${diagnostic.text}`,
    ), []);
  } finally {
    api.close();
  }
});
