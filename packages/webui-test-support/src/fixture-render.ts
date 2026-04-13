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

import { existsSync, readFileSync, readdirSync, writeFileSync, watch } from 'node:fs';
import { resolve } from 'node:path';
import { build, render } from '@microsoft/webui';

export interface RenderedFixture {
  /** Fixture directory name (e.g. "counter"). */
  name: string;
  /** Full rendered HTML including template IIFEs and hydration markers. */
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

  const buildOptions: Parameters<typeof build>[0] = { appDir: srcDir, plugin: 'webui' };
  if (fixtureConfig.css === 'link' || fixtureConfig.css === 'style' || fixtureConfig.css === 'module') {
    buildOptions.css = fixtureConfig.css;
  }

  const result = build(buildOptions);

  if (!result.protocol || result.protocol.length === 0) {
    throw new Error(
      `[fixture-render] build() returned empty protocol for fixture "${name}". ` +
      `Check ${srcDir}/index.html for syntax errors.`,
    );
  }

  let html = render(result.protocol, state, { plugin: 'webui' });

  const scriptTag = `<script src="/dist/${name}/element.js"></script>`;
  const bodyEnd = html.lastIndexOf('</body>');
  if (bodyEnd !== -1) {
    html = html.slice(0, bodyEnd) + scriptTag + html.slice(bodyEnd);
  } else {
    html += scriptTag;
  }

  return { name, html };
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

