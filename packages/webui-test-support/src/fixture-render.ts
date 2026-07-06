// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Builds and renders WebUI fixtures from real HTML templates.
 *
 * Each fixture directory may contain a `src/` subdirectory with real WebUI
 * template files (index.html, component HTML).  When present, this module
 * compiles the templates via `@microsoft/webui` build() and renders them
 * into full SSR HTML — replacing the need for hand-crafted fixture.html
 * files and manual TemplateMeta construction.
 */

import {
  cpSync,
  existsSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
  watch,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { build, render } from '@microsoft/webui';

const MARKER_SOURCE = `// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

`;

export interface RenderedFixture {
  /** Fixture directory name (e.g. "counter"). */
  name: string;
  /** Full rendered HTML including template metadata, condition closures, and hydration markers. */
  html: string;
}

export interface RenderFixturesOptions {
  /** Root directory containing fixture subdirectories. */
  fixturesRoot: string;
  /** Write rendered HTML to fixture directories as fixture.html (default: false). */
  writeFiles?: boolean;
  /** Watch src/ dirs for changes and re-render automatically (default: false). */
  watchMode?: boolean;
}

/** Build and render a single fixture from its src/ directory. */
function renderOne(fixturePath: string, name: string): RenderedFixture | null {
  const srcDir = resolve(fixturePath, 'src');
  if (!existsSync(resolve(srcDir, 'index.html'))) {
    return null;
  }

  const stateFile = resolve(fixturePath, 'state.json');
  const state = existsSync(stateFile)
    ? readFileSync(stateFile, 'utf-8')
    : '{}';

  const configFile = resolve(fixturePath, 'webui.config.json');
  const fixtureConfig: Record<string, string> = existsSync(configFile)
    ? JSON.parse(readFileSync(configFile, 'utf-8'))
    : {};

  const hasAuthoredEntry = existsSync(resolve(fixturePath, 'element.ts'));
  const authoredTags = hasAuthoredEntry
    ? collectDefinedTags(readFileSync(resolve(fixturePath, 'element.ts'), 'utf-8'))
    : new Set<string>();
  const authoredSource = hasAuthoredEntry ? createAuthoredSourceMirror(srcDir, authoredTags) : null;
  const buildOptions: Parameters<typeof build>[0] = hasAuthoredEntry
    ? { appDir: authoredSource?.fixturePath ?? fixturePath, entry: 'src/index.html', plugin: 'webui' }
    : { appDir: srcDir, plugin: 'webui' };
  if (fixtureConfig.css === 'link' || fixtureConfig.css === 'style' || fixtureConfig.css === 'module') {
    buildOptions.css = fixtureConfig.css;
  }

  const result = (() => {
    try {
      return build(buildOptions);
    } finally {
      if (authoredSource) {
        rmSync(authoredSource.rootPath, { recursive: true, force: true });
      }
    }
  })();

  if (!result.protocol || result.protocol.length === 0) {
    throw new Error(
      `[fixture-render] build() returned empty protocol for fixture "${name}". ` +
      `Check ${srcDir}/index.html for syntax errors.`,
    );
  }

  const renderEntry = hasAuthoredEntry ? 'src/index.html' : 'index.html';
  let html = render(result.protocol, state, { entry: renderEntry, plugin: 'webui' });

  // Fixtures with an authored element entry exercise interactive islands.
  // Fixtures without one use the shared framework root bootstrap to prove
  // HTML-only templates do not need empty component stubs.
  const scriptPath = hasAuthoredEntry
    ? `/dist/${name}/element.js`
    : '/dist/static-host.js';
  const scriptTag = `<script src="${scriptPath}"></script>`;
  const bodyEnd = html.lastIndexOf('</body>');
  if (bodyEnd !== -1) {
    html = html.slice(0, bodyEnd) + scriptTag + html.slice(bodyEnd);
  } else {
    html += scriptTag;
  }

  return { name, html };
}

interface AuthoredSourceMirror {
  rootPath: string;
  fixturePath: string;
}

function createAuthoredSourceMirror(srcDir: string, authoredTags: Set<string>): AuthoredSourceMirror {
  const rootPath = mkdtempSync(resolve(tmpdir(), 'webui-fixture-'));
  const fixturePath = resolve(rootPath, 'fixture');
  const mirroredSrcDir = resolve(fixturePath, 'src');
  cpSync(srcDir, mirroredSrcDir, { recursive: true });
  writeAuthoredComponentMarkers(mirroredSrcDir, authoredTags);
  return { rootPath, fixturePath };
}

function collectDefinedTags(source: string): Set<string> {
  const tags = new Set<string>();
  let cursor = 0;
  while (cursor < source.length) {
    const defineIndex = source.indexOf('.define(', cursor);
    if (defineIndex === -1) {
      break;
    }
    let valueStart = defineIndex + '.define('.length;
    while (valueStart < source.length && source[valueStart] === ' ') {
      valueStart++;
    }
    const quote = source[valueStart];
    if (quote !== '\'' && quote !== '"' && quote !== '`') {
      cursor = valueStart + 1;
      continue;
    }
    const valueEnd = source.indexOf(quote, valueStart + 1);
    if (valueEnd === -1) {
      break;
    }
    const tagName = source.slice(valueStart + 1, valueEnd);
    if (tagName.includes('-')) {
      tags.add(tagName);
    }
    cursor = valueEnd + 1;
  }
  return tags;
}

function writeAuthoredComponentMarkers(srcDir: string, authoredTags: Set<string>): void {
  const stack = [srcDir];
  while (stack.length > 0) {
    const dir = stack.pop();
    if (!dir) {
      continue;
    }
    const entries = readdirSync(dir, { withFileTypes: true });
    for (let i = 0; i < entries.length; i++) {
      const entry = entries[i];
      const path = resolve(dir, entry.name);
      if (entry.isDirectory()) {
        stack.push(path);
        continue;
      }
      if (!entry.isFile() || !entry.name.endsWith('.html')) {
        continue;
      }
      const tagName = entry.name.slice(0, -'.html'.length);
      if (!authoredTags.has(tagName)) {
        continue;
      }
      const tsPath = resolve(dir, `${tagName}.ts`);
      const jsPath = resolve(dir, `${tagName}.js`);
      if (!existsSync(tsPath) && !existsSync(jsPath)) {
        writeFileSync(tsPath, MARKER_SOURCE, 'utf-8');
      }
    }
  }
}

/**
 * Discovers fixture directories that contain a `src/` subdirectory with an
 * `index.html` entry, builds their templates via the WebUI pipeline, and
 * renders each with its state.json data.
 *
 * Returns a Map from fixture name to rendered HTML string.
 */
export function renderFixtures({
  fixturesRoot,
  writeFiles = false,
  watchMode = false,
}: RenderFixturesOptions): Map<string, RenderedFixture> {
  const results = new Map<string, RenderedFixture>();

  const dirs = readdirSync(fixturesRoot, { withFileTypes: true })
    .filter((e) => e.isDirectory() && e.name !== 'dist');

  for (const dir of dirs) {
    const fixturePath = resolve(fixturesRoot, dir.name);
    const fixture = renderOne(fixturePath, dir.name);
    if (!fixture) continue;

    results.set(dir.name, fixture);

    if (writeFiles) {
      writeFileSync(resolve(fixturePath, 'fixture.html'), fixture.html, 'utf-8');
    }
  }

  if (watchMode) {
    for (const dir of dirs) {
      const srcDir = resolve(fixturesRoot, dir.name, 'src');
      if (!existsSync(srcDir)) continue;

      watch(srcDir, { recursive: true }, (_event, _filename) => {
        try {
          const fixturePath = resolve(fixturesRoot, dir.name);
          const fixture = renderOne(fixturePath, dir.name);
          if (fixture) {
            results.set(dir.name, fixture);
            console.log(`  [watch] re-rendered ${dir.name}`);
          }
        } catch (err) {
          console.error(`  [watch] error re-rendering ${dir.name}:`, err);
        }
      });
    }
  }

  return results;
}
