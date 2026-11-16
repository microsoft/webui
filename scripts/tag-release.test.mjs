// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const script = fileURLToPath(new URL('./tag-release.sh', import.meta.url)).replaceAll('\\', '/');
const bash = process.platform === 'win32'
  ? join(dirname(dirname(dirname(execFileSync('git', ['--exec-path'], { encoding: 'utf8' }).trim()))), 'bin/bash.exe')
  : 'bash';

function fixture(t, { module = 'github.com/microsoft/webui/go/v3' } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'webui release tags '));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const work = join(root, 'work');
  const remote = join(root, 'origin.git');
  const global = join(root, 'global.gitconfig');
  writeFileSync(global, '');
  const env = {
    ...process.env,
    GIT_CONFIG_GLOBAL: global,
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_AUTHOR_NAME: 'Release fixture',
    GIT_AUTHOR_EMAIL: 'release@example.test',
    GIT_COMMITTER_NAME: 'Release fixture',
    GIT_COMMITTER_EMAIL: 'release@example.test',
    GIT_TERMINAL_PROMPT: '0',
    VALIDATION_MODE: 'false',
    RELEASE_TAG: 'v3.1.2',
  };
  for (const name of [
    'GIT_DIR', 'GIT_WORK_TREE', 'GIT_COMMON_DIR', 'GIT_INDEX_FILE',
    'GIT_OBJECT_DIRECTORY', 'GIT_ALTERNATE_OBJECT_DIRECTORIES',
    'GIT_CONFIG_COUNT', 'GIT_CONFIG_PARAMETERS', 'GIT_NAMESPACE',
  ]) delete env[name];
  function git(args, cwd = work) {
    return execFileSync('git', args, { cwd, env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  }
  git(['init', '--bare', '--template=', remote], root);
  git(['init', '-b', 'main', '--template=', work], root);
  git(['remote', 'add', 'origin', remote]);
  assert.equal(git(['remote', 'get-url', 'origin']), remote);
  writeFileSync(join(work, 'README.md'), 'historical source\n');
  git(['add', '.']);
  git(['commit', '-m', 'Historical release without Go']);
  const historical = git(['rev-parse', 'HEAD']);
  mkdirSync(join(work, 'go'));
  writeFileSync(join(work, 'go/go.mod'), `module ${module}\n\ngo 1.25.0\n`);
  git(['add', '.']);
  git(['commit', '-m', 'Release with Go']);
  const commit = git(['rev-parse', 'HEAD']);
  git(['push', 'origin', 'main']);
  env.RELEASE_COMMIT = commit;
  const remoteGit = args => git(['--git-dir', remote, ...args]);
  function remoteTag(tag) {
    const result = spawnSync('git', ['--git-dir', remote, 'rev-parse', '--verify', `refs/tags/${tag}^{commit}`],
      { env, encoding: 'utf8' });
    return result.status === 0 ? result.stdout.trim() : null;
  }
  function seed(tag, target = commit) {
    git(['-c', 'tag.gpgSign=false', 'tag', '-a', tag, target, '-m', `Existing ${tag}`]);
    git(['push', 'origin', `refs/tags/${tag}`]);
  }
  function run(overrides = {}) {
    // Safety invariant: every executed tag command has a fresh local bare origin.
    assert.equal(git(['remote', 'get-url', 'origin']), remote);
    return spawnSync(bash, [script], { cwd: work, env: { ...env, ...overrides }, encoding: 'utf8' });
  }
  return { root, work, remote, git, remoteGit, remoteTag, seed, run, commit, historical };
}

function success(output) {
  assert.equal(output.status, 0, `${output.stdout}\n${output.stderr}`);
}

for (const version of ['v3.1.2', 'v3.1.2-hotfix.4']) {
  test(`creates exact annotated root and Go tags for ${version}`, t => {
    const f = fixture(t);
    success(f.run({ RELEASE_TAG: version }));
    for (const tag of [version, `go/${version}`]) {
      assert.equal(f.remoteTag(tag), f.commit);
      assert.equal(f.remoteGit(['cat-file', '-t', `refs/tags/${tag}`]), 'tag');
    }
  });
}

test('a correct existing root never skips a missing Go tag; retries preserve both tag objects', t => {
  const f = fixture(t);
  f.seed('v3.1.2');
  const rootObject = f.remoteGit(['rev-parse', 'refs/tags/v3.1.2']);
  success(f.run());
  assert.equal(f.remoteTag('go/v3.1.2'), f.commit);
  const goObject = f.remoteGit(['rev-parse', 'refs/tags/go/v3.1.2']);
  success(f.run());
  assert.equal(f.remoteGit(['rev-parse', 'refs/tags/v3.1.2']), rootObject);
  assert.equal(f.remoteGit(['rev-parse', 'refs/tags/go/v3.1.2']), goObject);
});

for (const conflicting of ['v3.1.2', 'go/v3.1.2']) {
  test(`rejects divergent ${conflicting} before creating the other tag`, t => {
    const f = fixture(t);
    f.seed(conflicting, f.historical);
    const output = f.run();
    assert.notEqual(output.status, 0);
    assert.match(output.stderr, /points to .*not /);
    assert.equal(f.remoteTag(conflicting), f.historical);
    assert.equal(f.remoteTag(conflicting === 'v3.1.2' ? 'go/v3.1.2' : 'v3.1.2'), null);
  });
}

for (const racing of ['v3.1.2', 'go/v3.1.2']) {
  for (const sameCommit of [true, false]) {
    test(`concurrent ${racing} is ${sameCommit ? 'reused after revalidation' : 'rejected without retargeting'}`, t => {
      const f = fixture(t);
      // A one-shot local hook creates a lightweight tag just before the push.
      // Its object differs from our annotated tag even when its commit matches.
      mkdirSync(join(f.work, '.git/hooks'));
      writeFileSync(join(f.work, '.git/hooks/pre-push'), [
        '#!/bin/sh',
        'set -eu',
        'rm -- "$0"',
        'git --git-dir="$TAG_RACE_REMOTE" update-ref "refs/tags/$TAG_RACE_TAG" "$TAG_RACE_COMMIT"',
        '',
      ].join('\n'), { mode: 0o755 });
      const target = sameCommit ? f.commit : f.historical;
      const output = f.run({ TAG_RACE_REMOTE: f.remote, TAG_RACE_COMMIT: target, TAG_RACE_TAG: racing });
      if (sameCommit) {
        success(output);
        assert.match(output.stdout, /Concurrent release created/);
        assert.equal(f.remoteTag('go/v3.1.2'), f.commit);
      } else {
        assert.notEqual(output.status, 0);
        assert.match(output.stderr, /concurrent .*points to/);
        if (racing === 'v3.1.2') assert.equal(f.remoteTag('go/v3.1.2'), null);
      }
      assert.equal(f.remoteTag(racing), target);
    });
  }
}

test('historical no-Go commit creates root only despite Go being present in checkout', t => {
  const f = fixture(t);
  success(f.run({ RELEASE_COMMIT: f.historical }));
  assert.equal(f.remoteTag('v3.1.2'), f.historical);
  assert.equal(f.remoteTag('go/v3.1.2'), null);
});

test('wrong module identity and module major version fail before creating tags', t => {
  const wrong = fixture(t, { module: 'example.test/unapproved/go/v3' });
  const output = wrong.run();
  assert.notEqual(output.status, 0);
  assert.match(output.stderr, /expected github.com\/microsoft\/webui\/go\/v3/);
  assert.equal(wrong.remoteTag('v3.1.2'), null);
  assert.equal(wrong.remoteTag('go/v3.1.2'), null);
  const valid = fixture(t);
  assert.notEqual(valid.run({ RELEASE_TAG: 'v4.0.0' }).status, 0);
  assert.equal(valid.remoteTag('v4.0.0'), null);
});

test('validation runs, missing opt-in, and malformed metadata cannot create tags', t => {
  const f = fixture(t);
  for (const overrides of [
    { VALIDATION_MODE: 'true' },
    { VALIDATION_MODE: '' },
    { RELEASE_TAG: 'v3.1.2/invalid' },
    { RELEASE_COMMIT: 'HEAD' },
  ]) {
    assert.notEqual(f.run(overrides).status, 0);
    assert.equal(f.remoteTag('v3.1.2'), null);
    assert.equal(f.remoteTag('go/v3.1.2'), null);
  }
});

test('divergent local tag cannot be pushed when remote tag is absent', t => {
  const f = fixture(t);
  f.git(['tag', 'v3.1.2', f.historical]);
  const output = f.run();
  assert.notEqual(output.status, 0);
  assert.match(output.stderr, /local v3.1.2 points to/);
  assert.equal(f.remoteTag('v3.1.2'), null);
  assert.equal(f.remoteTag('go/v3.1.2'), null);
});

test('Azure artifact handoffs retain SDK checksums and the complete Go proxy tree', () => {
  const build = readFileSync(new URL('../.ado/pipelines/azure-pipelines-build.yml', import.meta.url), 'utf8');
  const cd = readFileSync(new URL('../.ado/pipelines/azure-pipelines-cd.yml', import.meta.url), 'utf8');
  for (const [directory, artifact] of [['sdk', 'standalone_sdk_archives'], ['go', 'go_module_proxy']]) {
    assert.ok(build.includes(`targetPath: '$(Build.SourcesDirectory)/publish/${directory}'\n        artifactName: ${artifact}`));
    assert.ok(cd.includes(`artifact: ${artifact}`));
    assert.ok(cd.includes(`SourceFolder: $(Pipeline.Workspace)/releaseBuild/${artifact}\n            Contents: '**'`));
    assert.ok(cd.includes(`targetPath: '$(Build.SourcesDirectory)/publish_artifacts_${directory}'\n            artifactName: ${artifact}`));
    assert.ok(cd.includes(`artifactName: ${artifact}\n              targetPath: $(Build.SourcesDirectory)/publish_artifacts_${directory}`));
  }
  assert.ok(build.includes('tar -czf "$AGENT_TEMPDIRECTORY/$proxy_archive" -C publish/go github.com'));
  assert.ok(cd.includes('$(Build.SourcesDirectory)/publish_artifacts_sdk/*.tar.gz.sha256'));
  assert.ok(cd.includes('$(Build.SourcesDirectory)/publish_artifacts_go/webui-go-proxy-*.tar.gz'));
  assert.ok(cd.includes("- ${{ if eq(parameters.validationMode, 'false') }}:\n      - stage: TagRelease"));
  assert.ok(cd.includes('filePath: $(Build.SourcesDirectory)/scripts/tag-release.sh'));
  assert.ok(cd.includes('VALIDATION_MODE: ${{ parameters.validationMode }}'));
  for (const job of ['QualityGate', 'BuildLinuxAssets', 'BuildMacOSAssets', 'BuildWindowsAssets', 'AssembleRelease']) {
    const section = build.split(`  - job: ${job}\n`)[1]?.split('\n  - job: ')[0];
    assert.ok(section?.includes("- task: GoTool@0\n      displayName: Use Go 1.25\n      inputs:\n        version: '1.25.0'"), job);
  }
});
