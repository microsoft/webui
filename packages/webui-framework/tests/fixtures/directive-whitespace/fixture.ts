// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { build, Protocol } from '../../../../webui/dist/index.js';
import { buildFixtureEntries } from '@microsoft/webui-test-support/fixture-build';
import { expect, type Page } from '@playwright/test';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { TemplateMeta } from '../../../src/template-types.js';
import type { DirectiveCase } from './sources.js';
import type { WhitespaceItem } from './test-directive-whitespace.js';

const here = dirname(fileURLToPath(import.meta.url));
const tag = 'test-directive-whitespace';
const authoredFile = `${tag}.ts`;

const items: WhitespaceItem[] = [
  {
    id: 'a', label: 'Alpha', visible: true, detail: true,
    children: [{ id: 'a1', label: 'A child 1' }, { id: 'a2', label: 'A child 2' }],
  },
  {
    id: 'b', label: 'Beta', visible: true, detail: false,
    children: [{ id: 'b1', label: 'B child 1' }, { id: 'b2', label: 'B child 2' }],
  },
];

export async function buildBrowserModule(workspace: string): Promise<string> {
  await buildFixtureEntries({
    fixturesRoot: dirname(here),
    entryFileName: authoredFile,
    outDir: resolve(workspace, 'bundle'),
    tsconfig: resolve(here, '../../../tsconfig.test.json'),
  });
  return readFileSync(
    resolve(workspace, 'bundle', 'directive-whitespace', `${tag}.js`), 'utf8',
  );
}

export function compileFixture(fixture: DirectiveCase, workspace: string) {
  const appDir = resolve(workspace, 'app');
  mkdirSync(appDir, { recursive: true });
  writeFileSync(resolve(appDir, authoredFile), readFileSync(resolve(here, authoredFile)));
  const sourcePath = resolve(appDir, `${tag}.html`);
  writeFileSync(sourcePath, fixture.source, 'utf8');
  expect(readFileSync(sourcePath)).toEqual(Buffer.from(fixture.source, 'utf8'));
  writeFileSync(resolve(appDir, 'index.html'),
    `<!doctype html><html><head><meta charset="utf-8"></head><body><${tag}></${tag}></body></html>`);
  const built = build({ appDir, plugin: 'webui' });
  const protocol = new Protocol(built.protocol, { plugin: 'webui' });
  const published = JSON.parse(protocol.renderComponentTemplates([tag], '')) as {
    templates: Record<string, TemplateMeta>;
  };
  const meta = published.templates[tag];
  expect(meta).toBeDefined();
  const html = protocol.render({
    enabled: true, ready: true, label: 'Action', selected: '', items,
    message: { enabled: true, ready: true, label: 'Action' },
  }).toString('utf8');
  return { meta, html };
}

export function createWorkspace(): string {
  const workspace = resolve(here, `.generated-${process.pid}`);
  mkdirSync(workspace, { recursive: true });
  return workspace;
}

export function removeWorkspace(workspace: string): void {
  rmSync(workspace, { recursive: true, force: true });
}

export async function loadSsr(page: Page, html: string, module: string): Promise<void> {
  await page.route('**/directive-whitespace/fixture.html', route =>
    route.fulfill({ contentType: 'text/html', body: html }));
  await page.route('**/directive-whitespace/component.js', route =>
    route.fulfill({ contentType: 'text/javascript', body: module }));
  await page.goto('/directive-whitespace/fixture.html');
  await page.evaluate(() => {
    const host = document.querySelector('test-directive-whitespace');
    if (!host?.shadowRoot) throw new Error('SSR must provide a parsed declarative shadow root');
    if (customElements.get('test-directive-whitespace')) throw new Error('Component registered before capture');
    window.__directiveWhitespaceSsr = Array.from(host.shadowRoot.querySelectorAll('*'));
    window.__directiveWhitespaceSsrParents = window.__directiveWhitespaceSsr
      .map(element => element.parentNode);
    window.__directiveWhitespaceKeyedSsr = new Map(
      Array.from(host.shadowRoot.querySelectorAll('[data-id]'), element => [
        `${element.tagName}.${element.className}:${element.getAttribute('data-id')}`,
        element,
      ]),
    );
  });
}

export async function hydrate(page: Page): Promise<void> {
  await page.evaluate(async moduleUrl => { await import(moduleUrl); },
    '/directive-whitespace/component.js');
}

export async function expectOriginalElements(page: Page): Promise<void> {
  expect(await page.evaluate(() => {
    const host = document.querySelector('test-directive-whitespace');
    const current = Array.from(host?.shadowRoot?.querySelectorAll('*') ?? []);
    return current.length === window.__directiveWhitespaceSsr.length
      && current.every((element, index) => element === window.__directiveWhitespaceSsr[index]
        && element.parentNode === window.__directiveWhitespaceSsrParents[index]);
  })).toBe(true);
}
