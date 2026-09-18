// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { TreeItem } from './element.js';
import type { Scenario } from './performance-client.js';

/** A type-only reference keeps the native renderer out of ordinary E2E startup. */
type RenderFixtures = typeof import('@microsoft/webui-test-support/fixture-render').renderFixtures;

const FIXTURE = 'recursive-fragments';
const BUNDLE_PATH = `/dist/${FIXTURE}/element.js`;
const SSR_SCRIPT_TAG = `<script type="module" src="${BUNDLE_PATH}"></script>`;
const here = dirname(fileURLToPath(import.meta.url));
const fixturesRoot = resolve(here, '..');
const projectionManifest = resolve(fixturesRoot, 'dist', 'webui-projection.json');

/** Explicit paths let an audited build use frozen inputs and an owned output manifest. */
export interface RenderContext {
  fixturesRoot: string;
  projectionManifest: string;
}

export interface SsrRender {
  scenario: Scenario;
  html: string;
  bytes: number;
}

export function buildItems(scenario: Scenario, suffix: string): TreeItem[] {
  let items: TreeItem[] = [];
  if (scenario.shape === 'wide') {
    for (let index = 0; index < scenario.size; index++) {
      items.push({ id: `node-${index}`, name: `Node ${index}${suffix}`, children: [] });
    }
    return items;
  }
  for (let index = scenario.size - 1; index >= 0; index--) {
    items = [{ id: `node-${index}`, name: `Node ${index}${suffix}`, children: items }];
  }
  return items;
}

/**
 * Renders the fixture through the real pipeline with a per-scenario state
 * override: same native build, same authored source mirror, same projection
 * manifest the fixture server used. No fixture file is rewritten and no
 * TemplateMeta is hand-built.
 */
export function renderSsr(
  scenario: Scenario,
  baseState: Record<string, unknown>,
  renderFixtures: RenderFixtures,
  context?: RenderContext,
): SsrRender {
  const state = JSON.stringify({
    ...baseState,
    title: 'Fragment SSR',
    items: buildItems(scenario, ''),
  });
  const rendered = renderFixtures({
    fixturesRoot: context?.fixturesRoot ?? fixturesRoot,
    fixtureNames: new Set([FIXTURE]),
    stateOverrides: { [FIXTURE]: state },
    projectionManifest: context?.projectionManifest ?? projectionManifest,
  }).get(FIXTURE);
  if (!rendered) throw new Error(`fixture "${FIXTURE}" produced no SSR output`);
  if (!rendered.html.includes(SSR_SCRIPT_TAG)) {
    throw new Error(
      `SSR HTML no longer ends with the expected entry tag ${SSR_SCRIPT_TAG}; ` +
      'adoption timing would be invalid because the bundle would run during parse.',
    );
  }
  // Removing the entry tag is what lets the browser parse the SSR DOM with no
  // framework code present, so the later injection times adoption alone.
  const html = rendered.html.replace(SSR_SCRIPT_TAG, '');
  return { scenario, html, bytes: Buffer.byteLength(rendered.html, 'utf8') };
}
