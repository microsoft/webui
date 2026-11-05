// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import { execFileSync, spawnSync } from "node:child_process";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { test } from "node:test";
import type { TestContext } from "node:test";

async function createFixture(t: TestContext) {
  const root = await mkdtemp(path.join(tmpdir(), "webui-ai-reference-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const packageRoot = path.join(root, "packages", "webui");
  const script = path.join(packageRoot, "scripts", "prepare-ai.js");
  const source = path.join(root, "docs", "ai.md");
  await mkdir(path.dirname(script), { recursive: true });
  await mkdir(path.dirname(source), { recursive: true });
  await copyFile(path.resolve("scripts", "prepare-ai.js"), script);
  await copyFile(
    path.resolve("package.json"),
    path.join(packageRoot, "package.json"),
  );
  await copyFile(path.resolve(".gitignore"), path.join(packageRoot, ".gitignore"));
  return { root, packageRoot, script, source };
}

const readReference = "require('node:fs').readFileSync(require.resolve('@microsoft/webui/ai.md'), 'utf8')";

async function markdownFiles(root: string): Promise<string[]> {
  const result: string[] = [];
  const pending = [""];
  while (pending.length) {
    const directory = pending.pop()!;
    for (const entry of await readdir(path.join(root, directory), { withFileTypes: true })) {
      if (entry.name.startsWith(".") || ["node_modules", "dist"].includes(entry.name)) continue;
      const relative = path.join(directory, entry.name);
      if (entry.isDirectory()) pending.push(relative);
      else if (entry.isFile() && entry.name.endsWith(".md")) result.push(relative);
    }
  }
  return result.sort();
}

async function copySourceDocs(destination: string): Promise<string[]> {
  const source = path.resolve("../../docs");
  const pages = await markdownFiles(source);
  for (const page of pages) {
    const target = path.join(destination, page);
    await mkdir(path.dirname(target), { recursive: true });
    await copyFile(path.join(source, page), target);
  }
  return pages;
}

function withoutLinkTargets(markdown: string): string {
  return markdown.replace(/\]\([^) \n]+(?:[ \t]+[^)\n]+)?\)/g, "](LINK)");
}

function proseLines(markdown: string): string[] {
  const result: string[] = [];
  let fence: string | undefined;
  for (const line of markdown.split("\n")) {
    const match = /^[ \t]*(`{3,}|~{3,})(.*)$/.exec(line);
    if (fence) {
      if (match && match[1][0] === fence[0] && match[1].length >= fence.length && !match[2].trim()) {
        fence = undefined;
      }
    } else if (match) {
      fence = match[1];
    } else {
      result.push(line);
    }
  }
  assert.equal(fence, undefined, "unclosed Markdown fence");
  return result;
}

function fencedBlocks(markdown: string): string[] {
  const result: string[] = [];
  let fence: string | undefined;
  let block = "";
  for (const line of markdown.split("\n")) {
    const match = /^[ \t]*(`{3,}|~{3,})(.*)$/.exec(line);
    if (fence) {
      block += `${line}\n`;
      if (match && match[1][0] === fence[0] && match[1].length >= fence.length && !match[2].trim()) {
        result.push(block);
        fence = undefined;
      }
    } else if (match) {
      fence = match[1];
      block = `${line}\n`;
    }
  }
  assert.equal(fence, undefined, "unclosed Markdown fence");
  return result;
}

function inlineCodeSpans(markdown: string): string[] {
  return proseLines(markdown).flatMap((line) =>
    Array.from(line.matchAll(/(`+)(.+?)\1/g), (match) => match[0])
  );
}

function htmlTagEnd(text: string, start: number): number {
  if (text.startsWith("<!--", start)) {
    const end = text.indexOf("-->", start + 4);
    return end < 0 ? -1 : end + 2;
  }
  let index = start + 1;
  if (text[index] === "/") index++;
  if (!/[a-z]/i.test(text[index] ?? "")) return -1;
  index++;
  while (/[a-z0-9-]/i.test(text[index] ?? "")) index++;
  if (!/[\s/>]/.test(text[index] ?? "")) return -1;
  let quote = "";
  for (; index < text.length; index++) {
    const character = text[index];
    if (quote) {
      if (character === quote) quote = "";
    } else if (character === '"' || character === "'") {
      quote = character;
    } else if (character === ">") {
      return index;
    }
  }
  return -1;
}

function headingSlug(markdown: string): string {
  let slug = "";
  const append = (character: string) => {
    if (/^[a-z0-9]$/i.test(character)) {
      slug += character.toLowerCase();
    } else if ((character === " " || character === "-" || character === "_") && !slug.endsWith("-")) {
      slug += "-";
    }
  };
  for (let index = 0; index < markdown.length;) {
    if (markdown[index] === "\\" && index + 1 < markdown.length) {
      append(markdown[index + 1]);
      index += 2;
      continue;
    }
    if (markdown[index] === "`") {
      let end = index + 1;
      while (markdown[end] === "`") end++;
      const delimiter = markdown.slice(index, end);
      let close = markdown.indexOf(delimiter, end);
      while (close >= 0 && (markdown[close - 1] === "`" || markdown[close + delimiter.length] === "`")) {
        close = markdown.indexOf(delimiter, close + delimiter.length);
      }
      if (close >= 0) {
        for (const character of markdown.slice(end, close)) append(character);
        index = close + delimiter.length;
        continue;
      }
    }
    if (markdown[index] === "_") {
      let end = index + 1;
      while (markdown[end] === "_") end++;
      if (/^[a-z0-9]$/i.test(markdown[index - 1] ?? "") && /^[a-z0-9]$/i.test(markdown[end] ?? "")) {
        append("_");
      }
      index = end;
      continue;
    }
    if (markdown[index] === "<") {
      const end = htmlTagEnd(markdown, index);
      if (end >= 0) {
        index = end + 1;
        continue;
      }
    }
    append(markdown[index++]);
  }
  return slug.endsWith("-") ? slug.slice(0, -1) : slug;
}

function headingIds(markdown: string): Set<string> {
  const ids = new Set<string>();
  for (const line of proseLines(markdown)) {
    for (const match of line.matchAll(/\bid=["']([^"']+)["']/g)) ids.add(match[1]);
    const heading = /^#{1,6}\s+(.+)$/.exec(line);
    if (!heading) continue;
    const slug = headingSlug(heading[1].replace(/\[([^\]]+)\]\([^)]+\)/g, "$1"));
    ids.add(slug);
  }
  return ids;
}

async function assertLocalLinks(packageRoot: string): Promise<number> {
  const docsRoot = path.join(packageRoot, "docs");
  const files = [path.join(packageRoot, "ai.md"), ...(await markdownFiles(docsRoot)).map(
    (page) => path.join(docsRoot, page),
  )];
  let checked = 0;
  for (const file of files) {
    const markdown = await readFile(file, "utf8");
    for (const line of proseLines(markdown)) {
      const text = line.replace(/(`+).*?\1/g, "");
      for (const match of text.matchAll(/\]\(([^) \t]+)(?:[ \t]+[^)]*)?\)/g)) {
        const target = match[1];
        if (/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(target)) continue;
        assert.ok(!target.startsWith("/"), `${file}: nonlocal link ${target}`);
        const [pathname, fragment] = target.split("#", 2);
        const targetFile = pathname
          ? path.resolve(path.dirname(file), decodeURIComponent(pathname.split("?")[0]))
          : file;
        const relative = path.relative(packageRoot, targetFile);
        assert.ok(!relative.startsWith("..") && !path.isAbsolute(relative), `${file}: ${target}`);
        assert.ok((await stat(targetFile)).isFile(), `${file}: ${target}`);
        if (fragment) {
          assert.ok(
            headingIds(await readFile(targetFile, "utf8")).has(decodeURIComponent(fragment)),
            `${file}: missing anchor ${target}`,
          );
        }
        checked++;
      }
    }
  }
  return checked;
}

function runPnpm(args: string[], cwd: string): string {
  const options = { cwd, encoding: "utf8" as const };
  if (process.platform === "win32") {
    return execFileSync(
      process.env.ComSpec ?? "cmd.exe",
      ["/d", "/s", "/c", `pnpm ${args.join(" ")}`],
      options,
    );
  }
  return execFileSync("pnpm", args, options);
}

test("heading IDs preserve inline code and ignore HTML markup", () => {
  const markdown = [
    "# `<for>` Loop Directive",
    "## The ``<template>`` Tag",
    "## A <em>formatted</em> heading",
    '## A <span title=">x">quoted</span> tag',
    "## A < 5 > 3",
    "## A <!-- > comment --> heading",
    "## `<script>alert(1)</script>` as text",
    "## <<script>script> remaining",
    "## `foo_bar` and foo_bar",
    "## _Italic_ and **Bold**",
    "## <https://example.com/a> link",
    "## <user@example.com> link",
    "## [A **linked** label](/some/path)",
  ].join("\n");
  assert.deepEqual([...headingIds(markdown)], [
    "for-loop-directive",
    "the-template-tag",
    "a-formatted-heading",
    "a-quoted-tag",
    "a-5-3",
    "a-heading",
    "scriptalert1script-as-text",
    "script-remaining",
    "foo-bar-and-foo-bar",
    "italic-and-bold",
    "httpsexamplecoma-link",
    "userexamplecom-link",
    "a-linked-label",
  ]);
});

test("dependency build hooks do not rewrite the package-only reference", async () => {
  const manifest = JSON.parse(await readFile(path.resolve("package.json"), "utf8"));
  for (const hook of ["prebuild", "build", "postbuild"]) {
    assert.doesNotMatch(
      manifest.scripts[hook] ?? "",
      /prepare-ai/,
      `${hook} must not race parallel dependency builds for ai.md`,
    );
  }
});

test("prepack refreshes the reference from the canonical document", async (t) => {
  const { root, packageRoot, script, source } = await createFixture(t);
  const manifest = JSON.parse(
    await readFile(path.join(packageRoot, "package.json"), "utf8"),
  );
  assert.ok(manifest.files.includes("ai.md"));
  assert.ok(manifest.files.includes("docs/"));
  assert.equal(manifest.scripts.prepack, "node scripts/prepare-ai.js");
  assert.equal(manifest.exports["./docs/*"], "./docs/*");
  const sourcePages = await copySourceDocs(path.join(root, "docs"));
  const canonical = await readFile(source, "utf8");
  execFileSync(process.execPath, [script], { cwd: root });
  const packaged = await readFile(path.join(packageRoot, "ai.md"), "utf8");
  assert.equal(withoutLinkTargets(packaged), withoutLinkTargets(canonical));
  assert.deepEqual(fencedBlocks(packaged), fencedBlocks(canonical));
  assert.deepEqual(inlineCodeSpans(packaged), inlineCodeSpans(canonical));
  assert.match(packaged, /\[Lazy UI\]\(\.\/docs\/learn\/lazy\.md\)/);
  assert.deepEqual(await markdownFiles(path.join(packageRoot, "docs")), sourcePages);
  for (const page of sourcePages) {
    const original = await readFile(path.join(root, "docs", page), "utf8");
    const copy = await readFile(path.join(packageRoot, "docs", page), "utf8");
    assert.equal(withoutLinkTargets(copy), withoutLinkTargets(original), page);
    assert.deepEqual(fencedBlocks(copy), fencedBlocks(original), page);
    assert.deepEqual(inlineCodeSpans(copy), inlineCodeSpans(original), page);
  }

  const updated = `${canonical}\nUpdated release guidance.\n`;
  await writeFile(source, updated);
  execFileSync(process.execPath, [script], { cwd: packageRoot });
  assert.equal(
    withoutLinkTargets(await readFile(path.join(packageRoot, "ai.md"), "utf8")),
    withoutLinkTargets(updated),
  );
});

test("the loader resolves hoisted and app-local versions without loading native code", async (t) => {
  const { root, packageRoot, script, source } = await createFixture(t);
  await writeFile(source, "Hoisted WebUI reference\n");
  await mkdir(path.join(root, "docs", "learn"), { recursive: true });
  await writeFile(path.join(root, "docs", "learn", "lazy.md"), "Hoisted lazy guide\n");
  execFileSync(process.execPath, [script]);
  const hoistedScope = path.join(root, "node_modules", "@microsoft");
  await mkdir(hoistedScope, { recursive: true });
  await symlink(packageRoot, path.join(hoistedScope, "webui"), "junction");
  const app = path.join(root, "apps", "my-app");
  await mkdir(app, { recursive: true });
  assert.equal(
    execFileSync(process.execPath, ["-p", readReference], {
      cwd: app,
      encoding: "utf8",
    }),
    "Hoisted WebUI reference\n\n",
  );
  assert.equal(
    execFileSync(process.execPath, ["-p",
      "require('node:fs').readFileSync(require.resolve('@microsoft/webui/docs/learn/lazy.md'), 'utf8')"], {
      cwd: app,
      encoding: "utf8",
    }),
    "Hoisted lazy guide\n\n",
  );

  const localPackage = path.join(app, "node_modules", "@microsoft", "webui");
  await mkdir(localPackage, { recursive: true });
  await copyFile(
    path.join(packageRoot, "package.json"),
    path.join(localPackage, "package.json"),
  );
  await writeFile(path.join(localPackage, "ai.md"), "App-local WebUI reference\n");
  await mkdir(path.join(localPackage, "docs", "learn"), { recursive: true });
  await writeFile(path.join(localPackage, "docs", "learn", "lazy.md"), "App-local lazy guide\n");
  assert.equal(
    execFileSync(process.execPath, ["-p", readReference], {
      cwd: app,
      encoding: "utf8",
    }),
    "App-local WebUI reference\n\n",
  );
  assert.equal(
    execFileSync(process.execPath, ["-p",
      "require('node:fs').readFileSync(require.resolve('@microsoft/webui/docs/learn/lazy.md'), 'utf8')"], {
      cwd: app,
      encoding: "utf8",
    }),
    "App-local lazy guide\n\n",
  );
});

test("preparing a reference fails when the canonical document is missing", async (t) => {
  const { root, packageRoot, script } = await createFixture(t);
  await writeFile(path.join(packageRoot, "ai.md"), "Stale guidance");
  const result = spawnSync(process.execPath, [script], {
    cwd: root,
    encoding: "utf8",
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /ENOENT/);
  assert.match(result.stderr, /ai\.md/);
});

test("packaged links preserve code examples and refresh removed pages", async (t) => {
  const { root, packageRoot, script, source } = await createFixture(t);
  const docs = path.join(root, "docs");
  await mkdir(path.join(docs, "learn"), { recursive: true });
  const canonical = [
    "# Reference",
    "[Guide](/learn/lazy#opening)",
    "[Titled](/learn/lazy#opening \"Open\")",
    "[Inline heading](/learn/lazy#lazy-ui)",
    "`[inline](/not-a-page)`",
    "``[inline](/also-not-a-page)``",
    "```markdown",
    "[example](/not-a-page)",
    "```",
    "~~~markdown",
    "[example](/also-not-a-page)",
    "~~~",
    "",
  ].join("\n");
  await writeFile(source, canonical);
  await writeFile(path.join(docs, "learn", "lazy.md"), "# Opening\n[Home](../ai.md)\n## `<lazy>` UI\n");
  await writeFile(path.join(docs, "learn", "obsolete.md"), "# Obsolete\n");
  await mkdir(path.join(docs, ".site"), { recursive: true });
  await writeFile(path.join(docs, ".site", "hidden.md"), "# Site only\n");
  execFileSync(process.execPath, [script]);

  const packaged = await readFile(path.join(packageRoot, "ai.md"), "utf8");
  assert.match(packaged, /\[Guide\]\(\.\/docs\/learn\/lazy\.md#opening\)/);
  assert.match(packaged, /\[Titled\]\(\.\/docs\/learn\/lazy\.md#opening "Open"\)/);
  assert.match(packaged, /\[Inline heading\]\(\.\/docs\/learn\/lazy\.md#lazy-ui\)/);
  assert.match(packaged, /`\[inline\]\(\/not-a-page\)`/);
  assert.match(packaged, /``\[inline\]\(\/also-not-a-page\)``/);
  assert.match(packaged, /```\w*\n\[example\]\(\/not-a-page\)\n```/);
  assert.match(packaged, /~~~\w*\n\[example\]\(\/also-not-a-page\)\n~~~/);
  assert.match(
    await readFile(path.join(packageRoot, "docs", "learn", "lazy.md"), "utf8"),
    /\[Home\]\(\.\.\/ai\.md\)/,
  );
  assert.ok((await assertLocalLinks(packageRoot)) > 0);
  await assert.rejects(stat(path.join(packageRoot, "docs", ".site", "hidden.md")), { code: "ENOENT" });

  await rm(path.join(docs, "learn", "obsolete.md"));
  execFileSync(process.execPath, [script]);
  await assert.rejects(stat(path.join(packageRoot, "docs", "learn", "obsolete.md")), { code: "ENOENT" });
});

test("an unresolved local guide fails prepack without replacing the prior bundle", async (t) => {
  const { packageRoot, script, source } = await createFixture(t);
  await writeFile(source, "Previous reference\n");
  execFileSync(process.execPath, [script]);
  await writeFile(source, "[Missing](/learn/missing)\n");
  const result = spawnSync(process.execPath, [script], { encoding: "utf8" });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Cannot resolve documentation link/);
  assert.match(result.stderr, /\/learn\/missing/);
  assert.equal(await readFile(path.join(packageRoot, "ai.md"), "utf8"), "Previous reference\n");
  assert.equal(
    await readFile(path.join(packageRoot, "docs", "ai.md"), "utf8"),
    "Previous reference\n",
  );
});

test("a packed reference installs and all local documentation links resolve offline", async (t) => {
  const { root, packageRoot } = await createFixture(t);
  const sourcePages = await copySourceDocs(path.join(root, "docs"));
  const manifestPath = path.join(packageRoot, "package.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  delete manifest.devDependencies;
  delete manifest.optionalDependencies;
  delete manifest.peerDependencies;
  delete manifest.peerDependenciesMeta;
  manifest.scripts = { prepack: manifest.scripts.prepack };
  await writeFile(manifestPath, JSON.stringify(manifest, null, 2));
  runPnpm(["pack"], packageRoot);
  const archives = (await readdir(packageRoot)).filter((file) => file.endsWith(".tgz"));
  assert.equal(archives.length, 1);

  const app = path.join(root, "app");
  await mkdir(app);
  await writeFile(path.join(app, "package.json"), '{"name":"consumer","version":"1.0.0","private":true}\n');
  await copyFile(path.join(packageRoot, archives[0]), path.join(app, "reference.tgz"));
  runPnpm(["add", "--offline", "--ignore-scripts", "./reference.tgz"], app);

  const reference = execFileSync(process.execPath, ["-p", "require.resolve('@microsoft/webui/ai.md')"], {
    cwd: app, encoding: "utf8",
  }).trim();
  const installed = path.dirname(reference);
  assert.deepEqual(await markdownFiles(path.join(installed, "docs")), sourcePages);
  const guide = execFileSync(process.execPath, ["-p", "require.resolve('@microsoft/webui/docs/learn/lazy.md')"], {
    cwd: app, encoding: "utf8",
  }).trim();
  assert.equal(await readFile(guide, "utf8"), await readFile(path.join(installed, "docs", "learn", "lazy.md"), "utf8"));
  assert.ok((await assertLocalLinks(installed)) > 200);
  assert.match(
    await readFile(path.join(installed, "docs", "start", "index.md"), "utf8"),
    /https:\/\/microsoft\.github\.io\/webui\/playground\//,
  );
});
