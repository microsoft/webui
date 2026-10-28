// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";

const docs = new URL("../../", import.meta.url);
const publicRoot = new URL("../public/", import.meta.url);
const manifest = new URL("../generated/redirect-paths.json", import.meta.url);

function relativeRoute(value) {
  if (typeof value !== "string" || !value.startsWith("/") || !value.endsWith("/") || value.includes("//")) {
    throw new Error(`Expected a slash-delimited documentation route, got ${JSON.stringify(value)}.`);
  }
  for (const character of value) {
    if (!"/-abcdefghijklmnopqrstuvwxyz0123456789".includes(character)) {
      throw new Error(`Unsupported character in documentation route ${JSON.stringify(value)}.`);
    }
  }
  return value.slice(1);
}

async function fileExists(file) {
  try {
    return (await stat(file)).isFile();
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

async function hasDocument(route) {
  return await fileExists(new URL(`${route}index.md`, docs))
    || await fileExists(new URL(`${route.slice(0, -1)}.md`, docs));
}

function escapeHtml(value) {
  return value.replaceAll("&", "&amp;").replaceAll('"', "&quot;")
    .replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

try {
  const config = JSON.parse(await readFile(new URL("../config.json", import.meta.url), "utf8"));
  const routes = JSON.parse(await readFile(new URL("../redirects.json", import.meta.url), "utf8"));
  if (!routes || typeof routes !== "object" || Array.isArray(routes)) {
    throw new Error("redirects.json must map legacy routes to canonical routes.");
  }
  const base = `/${relativeRoute(config.basePath)}`;
  const title = escapeHtml(config.site.title);
  const pages = [];
  for (const [from, to] of Object.entries(routes)) {
    const oldRoute = relativeRoute(from);
    const newRoute = relativeRoute(to);
    if (!oldRoute || await hasDocument(oldRoute)) {
      throw new Error(`Cannot redirect ${from}: it still has a source page.`);
    }
    if (!await hasDocument(newRoute)) {
      throw new Error(`Cannot redirect ${from}: ${to} has no Markdown source.`);
    }
    const destination = base + newRoute;
    const canonical = new URL(destination, config.site.url).href;
    pages.push({
      route: from,
      file: new URL(`${oldRoute}index.html`, publicRoot),
      html: `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Page moved | ${title}</title>
<meta name="description" content="This documentation page has moved.">
<link rel="canonical" href="${escapeHtml(canonical)}">
<script>location.replace(${JSON.stringify(destination)} + location.search + location.hash);</script>
<noscript><meta http-equiv="refresh" content="0;url=${destination}"></noscript>
</head>
<body><p>This page has moved to <a href="${destination}">${destination}</a>.</p></body>
</html>
`,
    });
  }
  let previous = [];
  try {
    previous = JSON.parse(await readFile(manifest, "utf8"));
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  if (!Array.isArray(previous)) throw new Error("The generated redirect manifest must be an array.");
  const obsolete = previous.filter(route => !Object.hasOwn(routes, route)).map(route => {
    const relative = relativeRoute(route);
    if (!relative) throw new Error("The generated redirect manifest cannot include the site root.");
    return new URL(`${relative}index.html`, publicRoot);
  });
  for (const page of pages) {
    await mkdir(new URL(".", page.file), { recursive: true });
    await writeFile(page.file, page.html, "utf8");
  }
  for (const file of obsolete) await rm(file, { force: true });
  await mkdir(new URL(".", manifest), { recursive: true });
  await writeFile(manifest, `${JSON.stringify(pages.map(page => page.route))}\n`, "utf8");
  console.log(`Prepared ${pages.length} documentation redirects.`);
} catch (error) {
  console.error(`[docs-redirects] ${error instanceof Error ? error.message : String(error)}\nhelp: Check redirects.json, its Markdown destinations, and the site URL/base path.`);
  process.exitCode = 1;
}
