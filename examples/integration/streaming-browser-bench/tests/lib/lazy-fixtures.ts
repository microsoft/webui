// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { build } from 'esbuild';
import { gzipSync } from 'node:zlib';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const FRAMEWORK_SRC = process.env.WEBUI_LAZY_HYDRATION_FRAMEWORK_SRC
  ? resolve(process.env.WEBUI_LAZY_HYDRATION_FRAMEWORK_SRC)
  : resolve(
    here,
    '..',
    '..',
    '..',
    '..',
    '..',
    'packages',
    'webui-framework',
    'src',
  );

export const TODO_TAG = 'bench-todo-item';
export const ITEM_COUNTS = [10, 1_000] as const;

/**
 * Which bundle to produce. Coordinator inclusion is a build-time (static
 * import) decision, not a runtime flag: the `visible` bundle additionally
 * imports the optional `visible-hydration-entry.ts` before defining the
 * component with `static override readonly hydration = 'visible'`; the `eager` bundle
 * never references it.
 */
export type LazyBenchMode = 'eager' | 'visible';

const TODO_TEMPLATE = {
  h: '<article><input type="checkbox"><span></span><small></small><strong></strong><time></time><button type="button">Toggle</button><button type="button">Delete</button></article>',
  tr: ['title', 'description', 'priority', 'due'],
  tx: [
    [[[0, 1], 0], [['title']]],
    [[[0, 2], 0], [['description']]],
    [[[0, 3], 0], [['priority']]],
    [[[0, 4], 0], [['due']]],
  ],
  eg: [
    ['click', [
      ['toggle', [], [0, 5]],
      ['remove', [], [0, 6]],
    ]],
  ],
} as const;

function entrySource(mode: LazyBenchMode): string {
  const optionalImport = mode === 'visible'
    ? "import './visible-hydration-entry.js';\n"
    : '';
  const hydrationField = mode === 'visible'
    ? "  static override readonly hydration = 'visible';\n\n"
    : '';
  return `
${optionalImport}import { WebUIElement } from './index.js';

function noteHydration(el, started) {
  const win = window;
  win.__benchHydrationCpu = (win.__benchHydrationCpu || 0) + performance.now() - started;
  if (!el.__benchCounted && el.$hydrated === true) {
    el.__benchCounted = true;
    win.__benchHydratedCount = (win.__benchHydratedCount || 0) + 1;
    win.__benchListenerCount =
      (win.__benchListenerCount || 0) + (el.$root?.cleanups?.length || 0);
  }
  const memory = performance.memory;
  if (memory && (!win.__benchPeakHeap || memory.usedJSHeapSize > win.__benchPeakHeap)) {
    win.__benchPeakHeap = memory.usedJSHeapSize;
  }
}

class BenchTodoItem extends WebUIElement {
${hydrationField}  connectedCallback() {
    const started = performance.now();
    super.connectedCallback();
    noteHydration(this, started);
  }

  $activateDeferredSSR(state) {
    const started = performance.now();
    super.$activateDeferredSSR(state);
    noteHydration(this, started);
  }

  toggle() {
    window.__benchInteractionCount = (window.__benchInteractionCount || 0) + 1;
  }

  remove() {
    window.__benchInteractionCount = (window.__benchInteractionCount || 0) + 1;
  }
}

window.__defineBenchTodo = function defineBenchTodo() {
  if (customElements.get('${TODO_TAG}')) return;
  BenchTodoItem.define('${TODO_TAG}');
};
`;
}

export interface LazyFixture {
  readonly mode: LazyBenchMode;
  readonly code: string;
  readonly minifiedBytes: number;
  readonly gzipBytes: number;
  /**
   * Input file paths (relative to the framework `src/` dir) esbuild reached
   * while bundling this fixture. Used to assert the eager bundle's module
   * graph never includes the coordinator, rather than inferring that from
   * byte size alone.
   */
  readonly inputs: readonly string[];
}

export async function buildLazyFixture(mode: LazyBenchMode): Promise<LazyFixture> {
  const result = await build({
    stdin: {
      contents: entrySource(mode),
      resolveDir: FRAMEWORK_SRC,
      loader: 'ts',
      sourcefile: 'lazy-hydration-fixture.ts',
    },
    bundle: true,
    write: false,
    minify: true,
    format: 'iife',
    platform: 'browser',
    target: 'es2022',
    define: { __WEBUI_DEV__: 'false' },
    supported: { 'import-attributes': true },
    legalComments: 'none',
    metafile: true,
  });
  const code = result.outputFiles[0].text;
  return {
    mode,
    code,
    minifiedBytes: Buffer.byteLength(code, 'utf-8'),
    gzipBytes: gzipSync(Buffer.from(code, 'utf-8')).length,
    inputs: Object.keys(result.metafile.inputs),
  };
}

export function lazyBaseHtml(): string {
  const bootstrap = {
    templates: { [TODO_TAG]: TODO_TEMPLATE },
    state: {
      title: 'Todo',
      description: 'A representative task with several bound fields',
      priority: 'high',
      due: '2030-01-01',
    },
  };
  return '<!doctype html><html><head><meta charset="utf-8">'
    + '<style>body{margin:0}bench-todo-item{display:block;height:72px}'
    + 'article{box-sizing:border-box;height:68px;padding:8px;border-bottom:1px solid #ccc}'
    + 'span,small,strong,time{display:inline-block;margin-inline:8px}</style>'
    + '</head><body>'
    + `<script id="webui-data" type="application/json">${JSON.stringify(bootstrap)}</script>`
    + '</body></html>';
}

export function todoRootsHtml(count: number): string {
  let html = '';
  for (let i = 0; i < count; i++) {
    html += `<${TODO_TAG} data-index="${i}"><article><input type="checkbox">`
      + `<span>Todo ${i}</span><small>Description ${i}</small><strong>high</strong>`
      + `<time>2030-01-01</time><button type="button">Toggle</button>`
      + `<button type="button">Delete</button></article></${TODO_TAG}>`;
  }
  return html;
}
