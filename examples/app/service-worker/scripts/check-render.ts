// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { readFile } from "fs/promises";
import assert from "node:assert/strict";
import { dirname, resolve } from "path";
import { fileURLToPath } from "url";
import { sanitizePayload } from "../src/payload.js";
import initWasm, { Protocol } from "../public/wasm/handler/webui_wasm_handler.js";

const here = dirname(fileURLToPath(import.meta.url));
const exampleRoot = resolve(here, "..");
const protocolPath = resolve(exampleRoot, "public/protocol.bin");
const wasmPath = resolve(exampleRoot, "public/wasm/handler/webui_wasm_handler_bg.wasm");
const themeCssPath = resolve(exampleRoot, "public/theme.css");
const apiFiles = ["shell", "hero", "metrics", "activity"];
const baseUrl = new URL("http://localhost:4175/");

interface WasmBoundaryDescriptor {
  instanceId: number;
  owner: string;
  name: string;
}

interface WasmStreamStep {
  bytes: Uint8Array;
  done: boolean;
  boundary?: WasmBoundaryDescriptor;
}

await initWasm({ module_or_path: await readFile(wasmPath) });

const protocol = new Protocol(
  new Uint8Array(await readFile(protocolPath)),
  "webui",
);

const payloads = new Map<string, ReturnType<typeof sanitizePayload>>();
for (const name of apiFiles) {
  const payload = JSON.parse(
    await readFile(resolve(exampleRoot, `public/api/${name}.json`), "utf-8"),
  );
  payloads.set(name, sanitizePayload(payload, `api/${name}.json`, baseUrl));
}

const themeCss = await readFile(themeCssPath, "utf-8");
const session = protocol.streamResponse("index.html", "/", {
  headInject: `<style>${themeCss}</style>`,
});
const decoder = new TextDecoder();
let html = "";
let step = session.start("{}") as WasmStreamStep;
html += decoder.decode(step.bytes);
while (!step.done) {
  const boundary = step.boundary;
  if (!boundary) {
    throw new Error("Streaming session returned no pending boundary");
  }
  const payload = payloads.get(boundary.name);
  if (!payload || boundary.owner !== "index.html") {
    throw new Error(`Unexpected boundary ${boundary.owner}/${boundary.name}`);
  }
  if (payload.entry !== `${boundary.name}-panel`) {
    throw new Error(`Boundary ${boundary.name} received state for ${payload.entry}`);
  }
  step = session.resume(
    boundary.instanceId,
    JSON.stringify(payload.state),
  ) as WasmStreamStep;
  html += decoder.decode(step.bytes);
}

for (const name of apiFiles) {
  if (!html.includes(`data-chunk="${name}"`)) {
    throw new Error(`Streaming response omitted ${name}`);
  }
}
if ((html.match(/class="card/g) ?? []).length < apiFiles.length) {
  throw new Error("Streaming response did not include every card");
}
if (!html.includes("<style>")) {
  throw new Error("Streaming response did not include component CSS");
}
if (html.includes("styles.css")) {
  throw new Error("Streaming response referenced the removed standalone stylesheet");
}

const bootstrapHtml = await readFile(resolve(exampleRoot, "public/index.html"), "utf-8");
assert.match(bootstrapHtml, /--color-brand-primary: #0078d4;/);
assert.doesNotMatch(bootstrapHtml, /WEBUI_THEME_(LIGHT|DARK)/);

assert.match(themeCss, /--color-brand-primary: #0078d4;/);
assert.doesNotMatch(themeCss, /WEBUI_THEME_(LIGHT|DARK)/);

assert.throws(
  () =>
    sanitizePayload(
      {
        entry: "hero-panel",
        state: { ctaHref: "javascript:alert(1)" },
      },
      "api/unsafe.json",
      baseUrl,
    ),
  /unsupported link scheme/,
);

console.log(`Validated ${apiFiles.length} service worker render chunks`);
