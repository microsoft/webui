// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import type { TestContext } from 'node:test';
import {
  build as buildPress, fixture as pressFixture, removeFixtureRoot, unusedPort, write,
} from '../../packages/webui-press/tests/native-fixture.js';
import { docsDist, startWebsite } from '../../packages/webui-press/tests/website-fixture.js';

const docs = path.dirname(docsDist);
const workspace = path.dirname(docs);
const webui = process.env.WEBUI_BINARY_PATH ??
  path.join(workspace, 'target/debug', process.platform === 'win32' ? 'webui.exe' : 'webui');
const tutorial = (name: string) =>
  fs.readFileSync(path.join(docs, 'start', `${name}.md`), 'utf8');
const installation = fs.readFileSync(path.join(docs, 'start/installation.md'), 'utf8');
const lessons = ['first-page', 'first-component', 'add-interactivity'] as const;

function lesson(name: (typeof lessons)[number]): string {
  return [installation, ...lessons.slice(0, lessons.indexOf(name) + 1).map(tutorial)].join('\n');
}

function filesFrom(markdown: string): Map<string, string> {
  const files = new Map<string, string>();
  const blocks = /^.*`((?:src\/|data\/)[^`\r\n]+|(?:package|tsconfig)\.json|state\.json|build-client\.mjs)`[^\r\n]*\r?\n\r?\n```[^\r\n]*\r?\n([\s\S]*?)^```/gm;
  for (const match of markdown.matchAll(blocks)) {
    const [, name, content] = match;
    assert.ok(!name.split('/').includes('..'), 'tutorial file must stay inside the project');
    const previous = files.get(name);
    if (name === 'package.json' && previous) {
      const current: Record<string, unknown> = JSON.parse(previous);
      const update: Record<string, unknown> = JSON.parse(content);
      files.set(name, JSON.stringify({ ...current, ...update }));
    } else {
      files.set(name, content);
    }
  }
  assert.ok(files.size > 0, 'tutorial must contain named, runnable file blocks');
  return files;
}

function fixtureFrom(t: TestContext, markdown: string): string {
  const parent = path.join(workspace, 'packages/webui-press/dist-test/sites');
  fs.mkdirSync(parent, { recursive: true });
  const root = fs.mkdtempSync(path.join(parent, 'tutorial-'));
  t.after(() => removeFixtureRoot(root));
  for (const [file, content] of filesFrom(markdown)) {
    write(path.join(root, file), content);
  }
  const framework = path.join(root, 'node_modules/@microsoft/webui-framework');
  fs.mkdirSync(path.dirname(framework), { recursive: true });
  fs.symlinkSync(path.join(workspace, 'packages/webui-framework'), framework, 'junction');
  return root;
}

function scriptsAt(root: string): Record<string, string> {
  const file = path.join(root, 'package.json');
  return fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, 'utf8')).scripts : {};
}

function nativeArgs(command: string): string[] {
  assert.ok(command.startsWith('webui '), `expected a documented CLI command: ${command}`);
  return command.split(/\s+/).slice(1);
}

function compile(root: string, command: string): void {
  execFileSync(webui, nativeArgs(command), { cwd: root, stdio: 'pipe', timeout: 30_000 });
  assert.ok(fs.statSync(path.join(root, 'dist/protocol.bin')).size > 0);
}

function bundle(root: string): void {
  const scripts = scriptsAt(root);
  assert.equal(scripts['build:client'], 'node build-client.mjs');
  assert.equal(scripts.build, 'npm run build:client && npm run build:protocol');
  execFileSync(process.execPath, ['build-client.mjs'], {
    cwd: root, stdio: 'pipe', timeout: 30_000,
  });
  assert.ok(fs.statSync(path.join(root, 'dist/index.js')).size > 0);
  compile(root, scripts['build:protocol']);
}

test('React bridge counter uses the executed tutorial sources', () => {
  const hello = filesFrom(lesson('add-interactivity'));
  const bridge = filesFrom(fs.readFileSync(
    path.join(workspace, 'docs/learn/react-comparison.md'), 'utf8'));
  for (const file of ['src/my-counter/my-counter.html', 'src/my-counter/my-counter.ts']) {
    assert.ok(hello.has(file));
    assert.equal(bridge.get(file), hello.get(file), `${file} must match the runnable counter`);
  }
});

test('installation scripts match the scriptless first app', () => {
  const hello = filesFrom(lesson('first-page'));
  const scripts = installation.match(/```json\r?\n([\s\S]*?)```/)?.[1];
  assert.ok(scripts);
  const packageJson = hello.get('package.json');
  assert.ok(packageJson);
  assert.deepEqual(JSON.parse(scripts).scripts, JSON.parse(packageJson).scripts);
  const interactive = filesFrom(lesson('add-interactivity')).get('package.json');
  assert.ok(interactive);
  assert.equal(JSON.parse(interactive).type, JSON.parse(packageJson).type);
  assert.equal(JSON.parse(interactive).private, JSON.parse(packageJson).private);
});

test('package-manager command tabs render and switch across the documentation', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const pages = [
    ['start/installation', 3],
    ['start/first-page', 1],
    ['start/first-component', 1],
    ['start/add-interactivity', 2],
    ['start/todo-app', 2],
    ['learn/how-it-works', 1],
    ['build/desktop', 2],
    ['build/hydration', 1],
    ['build/routing', 1],
    ['build/integrations/node', 1],
    ['reference/cli', 3],
    ['reference/webui-press', 1],
  ] as const;
  for (const [pathname, count] of pages) {
    await page.goto(`${server.origin}/webui/${pathname}/`);
    if (pathname === 'start/installation') {
      const policies = page.locator('details').filter({ hasText: 'Setup scripts blocked?' });
      await expect(policies).not.toHaveAttribute('open', '');
      await policies.locator('summary').click();
    }
    const groups = page.locator('webui-press-tabs').filter({
      has: page.locator('webui-press-tab', { hasText: /^npm$/ }),
    });
    await expect(groups).toHaveCount(count);
    await expect(page.locator('webui-press-tabs[sync="package-manager"][storage="local"]')).toHaveCount(count);
    const restored = pathname === 'start/installation' ? 'npm' : 'bun';
    await expect(groups.locator(`:scope > webui-press-tab[value="${restored}"][active]`))
      .toHaveCount(count);
    for (const group of await groups.all()) {
      const tabs = group.locator(':scope > webui-press-tab');
      const panels = group.locator(':scope > webui-press-tab-panel');
      await expect(tabs).toHaveText(['npm', 'pnpm', 'Yarn', 'Bun']);
      await expect(panels).toHaveCount(4);
      const commands = [
        ['npm', /^(?:npm|npx)\b/],
        ['pnpm', /^pnpm\b/],
        ['Yarn', /^yarn\b/],
        ['Bun', /^(?:bun|bunx)\b/],
      ] as const;
      for (const [index, [label, command]] of commands.entries()) {
        await tabs.nth(index).getByRole('button', { name: label, exact: true }).click();
        await expect(groups.locator(`:scope > webui-press-tab[value="${label.toLowerCase()}"][active]`),
          `${pathname}: selecting ${label} must synchronize every package-manager group`)
          .toHaveCount(count);
        await expect(panels.nth(index)).toBeVisible();
        await expect(panels.nth(index).locator('code-block').last().locator('code')).toContainText(command);
        await expect(group.locator(':scope > webui-press-tab-panel[active]')).toHaveCount(1);
        for (let other = 0; other < 4; other++) {
          if (other !== index) await expect(panels.nth(other)).toBeHidden();
        }
      }
    }
  }
  assert.deepEqual(errors, []);
});

test('code-only tabs have no outer code-block gaps while standalone blocks retain their spacing', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const javaScriptEnabled of [false, true]) {
    for (const colorScheme of ['light', 'dark'] as const) {
      const page = await browser.newPage({ javaScriptEnabled, colorScheme, viewport: { width: 390, height: 844 } });
      await page.goto(`${server.origin}/webui/start/installation/`);
      const group = page.locator('webui-press-tabs').first();
      const pre = group.locator('webui-press-tab-panel[active] > code-block:only-child > pre');
      await expect(pre).toHaveCSS('margin-top', '0px');
      await expect(pre).toHaveCSS('margin-bottom', '0px');
      await expect(pre).toHaveCSS('border-radius', '0px');
      await expect(pre).toHaveCSS('padding-top', '16px');
      const gaps = await group.evaluate(element => {
        const bar = element.shadowRoot?.querySelector('.tab-bar');
        const code = element.querySelector('webui-press-tab-panel[active] pre');
        if (!bar || !code) throw new Error('Missing tab geometry');
        return {
          top: code.getBoundingClientRect().top - bar.getBoundingClientRect().bottom,
          bottom: element.getBoundingClientRect().bottom - code.getBoundingClientRect().bottom,
        };
      });
      assert.ok(Math.abs(gaps.top) < 0.1, 'code must meet the tab bar without an empty strip');
      assert.ok(Math.abs(gaps.bottom - 1) < 0.1, 'only the tab container border may follow the code');
      await expect(page.locator('.doc-content > code-block > pre').first()).toHaveCSS('margin-bottom', '16px');
      await page.close();
    }
  }
});

test('installation keeps first-page setup separate from deep integrations', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ javaScriptEnabled: false });
  await page.goto(`${server.origin}/webui/start/installation/`);
  const content = page.locator('.doc-content');
  await expect(content.getByRole('heading', { level: 2 })).toHaveText([
    'Install the CLI #', 'Verify the CLI #', 'Create your first page #',
    'Installation policies #', 'Deep integrations #', 'Add what your app needs #',
  ]);
  for (const host of ['rust', 'node', 'dotnet', 'python', 'ffi', 'wasm']) {
    await expect(content.locator(`a[href="/webui/build/integrations/${host}"]`)).toHaveCount(1);
  }
  await expect(content.locator('a[href="/webui/start/first-page"]')).toBeVisible();
  await expect(content.locator('details[open]')).toHaveCount(0);
});

test('introductory page frontmatter builds with WebUI Press', t => {
  const f = pressFixture();
  t.after(() => removeFixtureRoot(f.root));
  write(f.configFile, JSON.stringify({ ...f.config, regions: {} }));
  const pages = [
    'start/index', 'learn/index', 'build/index', 'reference/index', 'learn/why', 'start/installation',
    'learn/react-comparison', 'learn/how-it-works',
    'build/performance', 'reference/best-practices',
    'start/first-page', 'start/first-component',
    'start/add-interactivity', 'start/todo-app',
  ];
  for (const [index, page] of pages.entries()) {
    const source = fs.readFileSync(path.join(workspace, 'docs', `${page}.md`), 'utf8');
    const frontmatter = source.match(/^---\r?\n[\s\S]*?\r?\n---/)?.[0];
    assert.ok(frontmatter, `${page} must describe its purpose`);
    write(path.join(f.site, 'content', `intro-${index}.md`),
      `${frontmatter}\n\n# Page metadata\n\nA readable introduction.\n`);
  }
  buildPress(f.site, '--show=content');
  for (const index of pages.keys()) {
    assert.ok(fs.existsSync(path.join(f.site, 'dist', `intro-${index}`, 'index.html')));
  }
});

async function serve(root: string, command: string) {
  const port = await unusedPort();
  const child = spawn(webui, [...nativeArgs(command), '--port', String(port)], {
    cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', data => { output += data; });
  child.stderr.on('data', data => { output += data; });
  const exited = once(child, 'exit');
  const stop = async () => {
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGINT');
    await exited;
  };
  const url = `http://127.0.0.1:${port}/`;
  let responseError = '';
  try {
    await expect.poll(async () => {
      assert.equal(child.exitCode, null, output);
      try {
        const response = await fetch(url, { headers: { Connection: 'close' } });
        if (!response.ok) responseError = await response.text();
        return response.status;
      } catch (error) {
        if (error instanceof TypeError && error.cause instanceof Error &&
            'code' in error.cause && error.cause.code === 'ECONNREFUSED') return 0;
        throw error;
      }
    }, { timeout: 30_000 }).toBe(200);
    return { url, stop };
  } catch (error) {
    await stop();
    throw new Error(`Tutorial preview failed:\n${output}\n${responseError}`, { cause: error });
  }
}

test('the lazy guide builds its documented region into a pack the CLI serves', async t => {
  const guide = fs.readFileSync(path.join(workspace, 'docs/learn/lazy.md'), 'utf8');
  const template = guide.match(/```html\r?\n<!-- app-shell\.html -->\r?\n([\s\S]*?)```/)?.[1];
  assert.ok(template, 'the guide must contain its component template');
  assert.match(guide, /import '\.\.\/dist\/lazy\.webui\.js';/);
  assert.match(guide, /await this\.\$rendered\(\);/);
  const owner = 'app-shell';
  const region = `${owner}::lazy:@0`;
  const parent = path.join(workspace, 'packages/webui-press/dist-test/sites');
  fs.mkdirSync(parent, { recursive: true });
  const root = fs.mkdtempSync(path.join(parent, 'lazy-guide-'));
  t.after(() => removeFixtureRoot(root));
  write(path.join(root, 'src/index.html'),
    `<!doctype html><html><head></head><body><${owner}></${owner}></body></html>`);
  write(path.join(root, `src/${owner}.html`), template);
  write(path.join(root, `src/${owner}.ts`), 'export {};');
  write(path.join(root, 'state.json'), JSON.stringify({ displayName: 'Ada', settingsOpen: false }));
  const server = await serve(root, 'webui serve ./src --state ./state.json --plugin webui');
  try {
    const html = await (await fetch(server.url)).text();
    assert.ok(!html.includes('<h2>Settings</h2>'), 'a closed body is not delivered with the page');
    const url = html.match(new RegExp(`"lp":\\{"${region}":"([^"]+)"\\}`))?.[1];
    assert.match(url ?? '', /^_webui\/packs\/app-shell\.[0-9a-f]{16}\.json$/);
    const response = await fetch(new URL(url!, server.url));
    assert.equal(response.status, 200);
    assert.match(response.headers.get('content-type') ?? '', /application\/json/);
    const pack = await response.json();
    assert.equal(pack.type, 'webui-pack');
    assert.equal(pack.fragment, region);
    assert.equal(pack.state, undefined);
    assert.match(pack.templates[region].h, /Settings/);
  } finally {
    await server.stop();
  }
});

test('Hello World scriptless commands use the documented files', async t => {
  const markdown = lesson('first-page');
  const root = fixtureFrom(t, markdown);
  const scripts = scriptsAt(root);
  const build = scripts.build ?? markdown.match(/^webui build .+$/m)?.[0];
  const dev = scripts.dev ?? markdown.match(/^webui serve .+$/m)?.[0];
  assert.ok(build);
  assert.ok(dev);
  compile(root, build);
  const server = await serve(root, dev);
  try {
    const html = await (await fetch(server.url)).text();
    assert.ok(!html.includes('<script'), 'the first page must not need a browser script');
    const browser = await chromium.launch({ headless: true });
    try {
      const page = await browser.newPage({ javaScriptEnabled: false });
      await page.goto(server.url);
      await expect(page.getByRole('heading', { name: 'Hello, Ada!' })).toBeVisible();
      await expect(page.getByText('This HTML was rendered on the server.')).toBeVisible();
      await expect(page).toHaveTitle('My first WebUI page');
    } finally {
      await browser.close();
    }
  } finally {
    await server.stop();
  }
});

test('the first component renders its data and paired CSS without browser JavaScript', async t => {
  const root = fixtureFrom(t, lesson('first-component'));
  const scripts = scriptsAt(root);
  compile(root, scripts.build);
  assert.equal(fs.existsSync(path.join(root, 'src/my-counter/my-counter.ts')), false);
  const server = await serve(root, scripts.dev);
  try {
    const browser = await chromium.launch({ headless: true });
    try {
      const page = await browser.newPage({ javaScriptEnabled: false });
      await page.goto(server.url);
      const counter = page.locator('my-counter');
      await expect(counter.getByText('Clicks: 0')).toBeVisible();
      await expect(counter).toHaveCSS('display', 'block');
      await expect(counter.getByRole('button')).toHaveCount(0);
    } finally {
      await browser.close();
    }
  } finally {
    await server.stop();
  }
});

test('Todo documented component templates compile', t => {
  const root = fixtureFrom(t, tutorial('todo-app'));
  const command = scriptsAt(root)['build:protocol'] ??
    'webui build ./src --out ./dist --plugin webui';
  compile(root, command);
});

test('the interaction lesson adopts SSR and responds to clicks', async t => {
  const root = fixtureFrom(t, lesson('add-interactivity'));
  bundle(root);
  const server = await serve(root, scriptsAt(root).dev);
  try {
    const browser = await chromium.launch({ headless: true });
    try {
      const page = await browser.newPage();
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => {
        if (message.type() === 'warning' || message.type() === 'error') errors.push(message.text());
      });
      let releaseScripts: () => void = () => {};
      const scriptsReady = new Promise<void>(resolve => { releaseScripts = resolve; });
      await page.route('**/index.js', async route => {
        await scriptsReady;
        await route.continue();
      });
      await page.goto(server.url, { waitUntil: 'commit' });
      const counter = page.locator('my-counter');
      try {
        await expect(counter.getByText('Clicks: 0')).toBeVisible();
        const button = await counter.getByRole('button', { name: 'Increment' }).elementHandle();
        assert.ok(button);
        releaseScripts();
        await expect.poll(() => counter.evaluate(host =>
          (host as HTMLElement & { $ready?: boolean }).$ready)).toBe(true);
        assert.ok(await button.evaluate(el =>
          el === document.querySelector('my-counter')?.shadowRoot?.querySelector('button')));
        await button.click();
        await button.click();
        await expect(counter.getByText('Clicks: 2')).toBeVisible();
        assert.deepEqual(errors, []);
      } finally {
        releaseScripts();
      }
    } finally {
      await browser.close();
    }
  } finally {
    await server.stop();
  }
});

test('Todo documented app adds, toggles and deletes without losing count or identity', async t => {
  const root = fixtureFrom(t, tutorial('todo-app'));
  bundle(root);
  const server = await serve(root, scriptsAt(root).dev);
  try {
    const browser = await chromium.launch({ headless: true });
    try {
      const page = await browser.newPage();
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => {
        if (message.type() === 'warning' || message.type() === 'error') errors.push(message.text());
      });
      await page.goto(server.url);
      await expect.poll(() => page.locator('todo-app').evaluate(host =>
        (host as HTMLElement & { $ready?: boolean }).$ready)).toBe(true);
      const count = page.getByRole('status');
      await expect(count).toHaveText('Remaining tasks: 2');
      const writing = page.locator('todo-item').filter({ hasText: 'Write documentation' });
      const retained = await writing.elementHandle();
      assert.ok(retained);
      await writing.getByRole('button', { name: 'Toggle complete' }).click();
      await expect(writing).toHaveAttribute('state', 'done');
      await expect(count).toHaveText('Remaining tasks: 1');
      const input = page.getByRole('textbox', { name: 'New task' });
      await input.fill('  Review docs  ');
      await input.press('Enter');
      await expect(page.locator('todo-item')).toHaveCount(4);
      await expect(count).toHaveText('Remaining tasks: 2');
      await expect(input).toHaveValue('');
      await expect(input).toBeFocused();
      await page.locator('todo-item').filter({ hasText: 'Review docs' })
        .getByRole('button', { name: 'Delete' }).click();
      await expect(count).toHaveText('Remaining tasks: 1');
      await page.locator('todo-item').filter({ hasText: 'Buy groceries' })
        .getByRole('button', { name: 'Delete' }).click();
      await expect(page.locator('todo-item')).toHaveCount(2);
      await expect(count).toHaveText('Remaining tasks: 1');
      assert.ok(await retained.evaluate(el => el.isConnected && el.id === '2'),
        'keyed rows must retain the existing component when an earlier row is removed');
      await input.fill('   ');
      await input.press('Enter');
      await expect(page.locator('todo-item')).toHaveCount(2);
      assert.deepEqual(errors, []);
    } finally {
      await browser.close();
    }
  } finally {
    await server.stop();
  }
});
