// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { readFile } from "fs/promises";
import assert from "node:assert/strict";
import { dirname, resolve } from "path";
import { fileURLToPath } from "url";
import { sanitizePayload } from "../src/payload.js";
import initWasm, { render } from "../public/wasm/handler/webui_wasm_handler.js";

const here = dirname(fileURLToPath(import.meta.url));
const exampleRoot = resolve(here, "..");
const protocolPath = resolve(exampleRoot, "public/protocol.bin");
const wasmPath = resolve(exampleRoot, "public/wasm/handler/webui_wasm_handler_bg.wasm");
const themeCssPath = resolve(exampleRoot, "public/theme.css");
const apiFiles = ["shell", "hero", "metrics", "activity"];
const baseUrl = new URL("http://localhost:4175/");

await initWasm({ module_or_path: await readFile(wasmPath) });

const protocol = new Uint8Array(await readFile(protocolPath));

for (const name of apiFiles) {
  const payload = JSON.parse(
    await readFile(resolve(exampleRoot, `public/api/${name}.json`), "utf-8"),
  );
  const sanitized = sanitizePayload(payload, `api/${name}.json`, baseUrl);
  let html = "";
  const onChunk = (chunk: string): void => {
    html += chunk;
  };
  render(
    protocol,
    JSON.stringify(sanitized.state),
    onChunk,
    { entry: sanitized.entry, requestPath: "/", plugin: "webui" },
  );
  if (!html.includes("card")) {
    throw new Error(`Rendered ${sanitized.entry} did not include expected card markup`);
  }
  if (!html.includes("<style>")) {
    throw new Error(`Rendered ${sanitized.entry} did not include component CSS`);
  }
  if (html.includes("styles.css")) {
    throw new Error(`Rendered ${sanitized.entry} referenced the removed standalone stylesheet`);
  }
}

const bootstrapHtml = await readFile(resolve(exampleRoot, "public/index.html"), "utf-8");
assert.match(bootstrapHtml, /--color-brand-primary: #0078d4;/);
assert.doesNotMatch(bootstrapHtml, /WEBUI_THEME_(LIGHT|DARK)/);

const themeCss = await readFile(themeCssPath, "utf-8");
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
