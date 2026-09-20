// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { buildSync } from 'esbuild';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const workspace = path.resolve(import.meta.dirname, '../../../..');
export const binary = process.env.WEBUI_PRESS_BINARY ??
  path.join(workspace, 'target/debug', process.platform === 'win32' ? 'webui-press.exe' : 'webui-press');

export function write(file: string, content: string): void {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, content);
}

export function fixture() {
  const parent = path.join(workspace, 'crates/webui-press/dist-test/sites');
  fs.mkdirSync(parent, { recursive: true });
  const root = fs.mkdtempSync(path.join(parent, 'show-mode-'));
  const site = path.join(root, 'site');
  const pkg = path.join(root, 'node_modules/@fixture/catalog');
  const configFile = path.join(site, '.webui-press/config.json');
  const template = path.join(pkg, 'components/test-catalog-button/test-catalog-button');
  write(`${template}.html`,
    '<button @click="{increment()}">{{label}} {{count}}</button><slot></slot>');
  write(`${template}.css`, ':host { display: block; } button { font: inherit; }');
  write(`${template}.json`, '{"description":"Synthetic catalog metadata"}');
  write(`${template}.spec.ts`, 'throw new Error("Specs must not be bundled");');
  write(`${template}.ts`, `
    import { WebUIElement, attr, observable } from '@microsoft/webui-framework';
    export class CatalogButton extends WebUIElement {
      @attr label = '';
      @observable count = 0;
      increment() { this.count += 1; }
    }
    CatalogButton.define('test-catalog-button');
  `);
  buildSync({
    entryPoints: [`${template}.ts`],
    outfile: path.join(pkg, 'dist/components/test-catalog-button/test-catalog-button.js'),
    format: 'esm',
    tsconfigRaw: { compilerOptions: { experimentalDecorators: true, useDefineForClassFields: false } },
  });
  write(path.join(pkg, 'package.json'), JSON.stringify({
    name: '@fixture/catalog', version: '1.0.0', type: 'module',
    customElements: '../must-not-read.json',
    exports: {
      './button.js': './dist/components/test-catalog-button/test-catalog-button.js',
      './template-webui.html': '../must-not-read.html',
    },
  }));
  const text = path.join(pkg, 'components/test-catalog-text/test-catalog-text');
  write(`${text}.html`, '<span>{{catalogMessage}} <slot></slot></span>');
  write(`${text}.css`, ':host { display: block; }');
  write(`${text}.json`, '{"description":"Scriptless catalog component"}');
  write(`${text}.md`, '# Scriptless catalog component');
  write(`${text}.spec.ts`, 'throw new Error("Specs are not component scripts");');
  for (const name of ['webui', 'webui-framework']) {
    const link = path.join(site, 'node_modules/@microsoft', name);
    fs.mkdirSync(path.dirname(link), { recursive: true });
    fs.symlinkSync(path.join(workspace, 'packages', name), link, 'junction');
  }
  const examples = `
<edge-hub-header></edge-hub-header>
<edge-side-pane></edge-side-pane>
<test-catalog-button label="Preview" :count="{{count}}"><span>Slotted example</span></test-catalog-button>
<test-catalog-text>Static slot</test-catalog-text>

## API

| Property | Default |
| --- | --- |
| count | 2 |

\`\`\`html
<test-catalog-button label="Preview"></test-catalog-button>
\`\`\`

<script type="module" bundle>
import '@fixture/catalog/button.js';
import '#gallery/preview.ts';
</script>
`;
  for (const layout of ['doc', 'page', 'full', 'home']) {
    write(path.join(site, 'content', `${layout}.md`),
      `---\nlayout: ${layout}\ntitle: ${layout} example\ndescription: Example metadata\n---\n\n# ${layout} example\n${examples}`);
  }
  write(path.join(site, 'content/local/edge-hub-header/edge-hub-header.html'),
    '<header>Authored header example</header>');
  write(path.join(site, 'content/local/edge-side-pane/edge-side-pane.html'),
    '<aside>Authored side pane example</aside>');
  write(path.join(site, 'theme.css'), ':root { --docs-color-brand: #0067b8; }');
  write(path.join(site, 'public/logo.svg'),
    fs.readFileSync(path.join(workspace, 'docs/.webui-press/public/logo.svg'), 'utf8'));
  write(path.join(site, 'shell.ts'), 'throw new Error("Shell region must be inactive");');
  write(path.join(site, '.webui-press/components/fixture-preview/preview.ts'),
    'document.documentElement.dataset.galleryAlias = "ready";');
  const config = {
    site: { title: 'Content gallery' }, basePath: '/fixture/',
    contentDir: './content', outDir: './dist', publicDir: './public',
    components: ['@fixture/catalog', './content/local'],
    // Projection analyzes original decorators, before JavaScript transformation.
    bundler: { alias: {
      '@fixture/catalog/button.js': `${template}.ts`,
      '#gallery': './components/fixture-preview',
    } },
    css: './theme.css', state: { count: 2, catalogMessage: 'Static catalog content' }, nav: [], sidebar: [],
    head: [{ tag: 'meta', attrs: { name: 'fixture-head', content: 'preserved' } }],
    footer: { html: 'Press footer' },
    customPages: { '/custom': { html: '<h1>Custom example</h1>' + examples, layout: 'full' } },
    regions: { 'site.announcement': { html: '<p>Shell announcement</p>', scriptFile: '../shell.ts' } },
  };
  write(configFile, JSON.stringify(config));
  return { root, site, configFile, config };
}

export function build(site: string, ...args: string[]): void {
  execFileSync(binary, ['build', ...args], { cwd: site, stdio: 'pipe', timeout: 60_000 });
}
