// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { mkdir, mkdtemp, readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

const sourceRoot = fileURLToPath(new URL("../../../docs/", import.meta.url));
const packageRoot = fileURLToPath(new URL("../", import.meta.url));
const website = "https://microsoft.github.io/webui/";
const siteOnly = new Set(["playground", "benchmarks/official-results.json"]);
const excludedDirectories = new Set(["dist", "node_modules"]);

async function findPages() {
  const pages = [];
  const pending = [""];
  while (pending.length) {
    const directory = pending.pop();
    const entries = await readdir(path.join(sourceRoot, directory), { withFileTypes: true });
    for (const entry of entries) {
      if (entry.name.startsWith(".") || excludedDirectories.has(entry.name)) continue;
      const relative = path.posix.join(directory, entry.name);
      if (entry.isDirectory()) pending.push(relative);
      else if (entry.isFile() && entry.name.endsWith(".md")) pages.push(relative);
      else if (entry.isSymbolicLink()) {
        throw new Error(`Documentation symlinks are not supported: ${relative}`);
      }
    }
  }
  return pages.sort();
}

function documentTarget(target, source, output, pages) {
  if (target.startsWith("#") || /^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(target)) return target;
  const split = target.search(/[?#]/);
  const pathname = split < 0 ? target : target.slice(0, split);
  const suffix = split < 0 ? "" : target.slice(split);
  const resolved = pathname.startsWith("/")
    ? path.posix.normalize(pathname.slice(1))
    : path.posix.normalize(path.posix.join(path.posix.dirname(source), pathname));
  if (resolved === ".." || resolved.startsWith("../")) {
    throw new Error(`Documentation link escapes docs/: ${target} in ${source}`);
  }

  const clean = resolved.replace(/\/+$/, "");
  const index = path.posix.join(clean, "index.md");
  const candidates = pathname.endsWith("/")
    ? [index, `${clean}.md`, clean]
    : [clean, `${clean}.md`, index];
  const page = candidates.find((candidate) => pages.has(candidate));
  if (!page) {
    if (siteOnly.has(clean)) {
      return `${website}${clean}${pathname.endsWith("/") ? "/" : ""}${suffix}`;
    }
    throw new Error(`Cannot resolve documentation link "${target}" in ${source}`);
  }

  const relative = path.posix.relative(path.posix.dirname(output), `docs/${page}`);
  return `${relative.startsWith(".") ? relative : `./${relative}`}${suffix}`;
}

function closingParenthesis(line, start) {
  let depth = 0;
  for (let i = start; i < line.length; i++) {
    if (line[i] === "\\") {
      i++;
    } else if (line[i] === "(") {
      depth++;
    } else if (line[i] === ")") {
      if (depth === 0) return i;
      depth--;
    }
  }
  return -1;
}

function closingCodeSpan(line, start, delimiter) {
  let end = line.indexOf(delimiter, start);
  while (end !== -1) {
    if (line[end - 1] !== "`" && line[end + delimiter.length] !== "`") return end;
    end = line.indexOf(delimiter, end + delimiter.length);
  }
  return -1;
}

function rewriteLine(line, source, output, pages) {
  let result = "";
  for (let i = 0; i < line.length;) {
    if (line[i] === "\\" && i + 1 < line.length) {
      result += line.slice(i, i + 2);
      i += 2;
    } else if (line[i] === "`") {
      let end = i + 1;
      while (line[end] === "`") end++;
      const delimiter = line.slice(i, end);
      const close = closingCodeSpan(line, end, delimiter);
      const next = close < 0 ? end : close + delimiter.length;
      result += line.slice(i, next);
      i = next;
    } else if (line[i] === "]" && line[i + 1] === "(") {
      const close = closingParenthesis(line, i + 2);
      if (close < 0) {
        result += line[i++];
        continue;
      }
      const body = line.slice(i + 2, close);
      const match = /^(\S+)([ \t]+.*)?$/.exec(body);
      if (!match) throw new Error(`Unsupported documentation link in ${source}: ${body}`);
      const target = documentTarget(match[1], source, output, pages);
      result += `](${target}${match[2] ?? ""})`;
      i = close + 1;
    } else {
      result += line[i++];
    }
  }
  return result;
}

function rewriteLinks(content, source, output, pages) {
  let fence;
  return content.split("\n").map((line) => {
    const match = /^[ \t]*(`{3,}|~{3,})(.*)$/.exec(line);
    if (fence) {
      if (match && match[1][0] === fence[0] && match[1].length >= fence.length && !match[2].trim()) {
        fence = undefined;
      }
      return line;
    }
    if (match) {
      fence = match[1];
      return line;
    }
    return rewriteLine(line, source, output, pages);
  }).join("\n");
}

const reference = await readFile(path.join(sourceRoot, "ai.md"), "utf8");
const pageNames = await findPages();
const pages = new Set(pageNames);
if (!pages.has("ai.md")) throw new Error("The canonical docs/ai.md is missing");
const stage = await mkdtemp(path.join(packageRoot, ".webui-reference-"));
try {
  for (const page of pageNames) {
    const content = page === "ai.md"
      ? reference
      : await readFile(path.join(sourceRoot, page), "utf8");
    const output = `docs/${page}`;
    const destination = path.join(stage, output);
    await mkdir(path.dirname(destination), { recursive: true });
    await writeFile(destination, rewriteLinks(content, page, output, pages));
    if (page === "ai.md") {
      await writeFile(path.join(stage, "ai.md"), rewriteLinks(content, page, "ai.md", pages));
    }
  }
  await rm(path.join(packageRoot, "docs"), { recursive: true, force: true });
  await rename(path.join(stage, "docs"), path.join(packageRoot, "docs"));
  await rename(path.join(stage, "ai.md"), path.join(packageRoot, "ai.md"));
} finally {
  await rm(stage, { recursive: true, force: true });
}
