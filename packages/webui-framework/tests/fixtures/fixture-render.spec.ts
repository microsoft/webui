// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test, type TestInfo } from '@playwright/test';
import { renderFixtures } from '@microsoft/webui-test-support/fixture-render';

function createFixtures(info: TestInfo): string {
  const fixturesRoot = info.outputPath('fixture-inputs');
  for (const name of ['first', 'second']) {
    const directory = resolve(fixturesRoot, name);
    mkdirSync(resolve(directory, 'src'), { recursive: true });
    writeFileSync(
      resolve(directory, 'src/index.html'),
      '<html><body><p>{{value}}</p></body></html>',
    );
    writeFileSync(resolve(directory, 'state.json'), JSON.stringify({ value: name }));
  }
  return fixturesRoot;
}

test.afterEach(({}, info) => {
  rmSync(info.outputPath('fixture-inputs'), { recursive: true, force: true });
});

test('fixture rendering defaults discover all sources and leave files unchanged', ({}, info) => {
  const fixturesRoot = createFixtures(info);
  mkdirSync(resolve(fixturesRoot, 'manual'), { recursive: true });
  const rendered = renderFixtures({ fixturesRoot });
  expect([...rendered.keys()].sort()).toEqual(['first', 'second']);
  for (const name of ['first', 'second']) {
    expect(rendered.get(name)?.html).toContain(`<p>${name}</p>`);
    expect(existsSync(resolve(fixturesRoot, name, 'fixture.html'))).toBe(false);
    expect(JSON.parse(readFileSync(resolve(fixturesRoot, name, 'state.json'), 'utf8')))
      .toEqual({ value: name });
  }
});

test('fixture selection does not compile or read state from excluded fixtures', ({}, info) => {
  const fixturesRoot = createFixtures(info);
  writeFileSync(
    resolve(fixturesRoot, 'second/src/index.html'),
    '<render fragment="undeclared"></render>',
  );
  writeFileSync(resolve(fixturesRoot, 'second/state.json'), '{');
  const rendered = renderFixtures({ fixturesRoot, fixtureNames: new Set(['first']) });
  expect([...rendered.keys()]).toEqual(['first']);
  expect(rendered.get('first')?.html).toContain('<p>first</p>');
  expect(renderFixtures({ fixturesRoot, fixtureNames: new Set() }).size).toBe(0);
});

test('state overrides apply only to the selected fixture and never replace its state file', ({}, info) => {
  const fixturesRoot = createFixtures(info);
  const stateOverrides = { first: JSON.stringify({ value: 'override' }) };
  const rendered = renderFixtures({ fixturesRoot, stateOverrides });
  expect(rendered.get('first')?.html).toContain('<p>override</p>');
  expect(rendered.get('second')?.html).toContain('<p>second</p>');
  expect(stateOverrides).toEqual({ first: '{"value":"override"}' });
  expect(readFileSync(resolve(fixturesRoot, 'first/state.json'), 'utf8')).toBe('{"value":"first"}');

  const empty = renderFixtures({
    fixturesRoot, fixtureNames: new Set(['first']), stateOverrides: { first: '{}' },
  });
  expect(empty.get('first')?.html).toContain('<p></p>');
  expect(() => renderFixtures({
    fixturesRoot, fixtureNames: new Set(['first']), stateOverrides: { first: '{' },
  })).toThrow(/State JSON error/);
});
