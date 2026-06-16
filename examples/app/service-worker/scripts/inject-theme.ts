// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { readFile, writeFile } from "fs/promises";
import { dirname, resolve } from "path";
import { fileURLToPath } from "url";
import themeJson from "@microsoft/webui-examples-theme/tokens.json";
import initWasm, { protocol_tokens } from "../public/wasm/handler/webui_wasm_handler.js";

interface ThemeFile {
  themes: Record<string, Record<string, string>>;
}

const here = dirname(fileURLToPath(import.meta.url));
const exampleRoot = resolve(here, "..");
const protocolPath = resolve(exampleRoot, "public/protocol.bin");
const bootstrapPath = resolve(exampleRoot, "src/bootstrap.html");
const indexPath = resolve(exampleRoot, "public/index.html");
const themeCssPath = resolve(exampleRoot, "public/theme.css");
const wasmPath = resolve(exampleRoot, "public/wasm/handler/webui_wasm_handler_bg.wasm");
const themeFile: ThemeFile = themeJson;

await initWasm({ module_or_path: await readFile(wasmPath) });

const protocol = new Uint8Array(await readFile(protocolPath));
const requiredTokens = protocol_tokens(protocol) as string[];
const html = await readFile(bootstrapPath, "utf-8");
const lightCss = resolveTheme("light", requiredTokens);
const darkCss = resolveTheme("dark", requiredTokens);
const themed = html
  .replace("<!--WEBUI_THEME_LIGHT-->", lightCss)
  .replace("<!--WEBUI_THEME_DARK-->", darkCss);

if (themed === html) {
  throw new Error("Theme placeholders were not replaced in src/bootstrap.html");
}

await writeFile(indexPath, themed);
await writeFile(themeCssPath, buildThemeStylesheet(lightCss, darkCss));
console.log(`Injected ${requiredTokens.length} protocol token(s) into public/index.html`);

function resolveTheme(themeName: string, requiredTokens: string[]): string {
  const theme = themeFile.themes[themeName];
  if (!theme) {
    throw new Error(`Theme '${themeName}' is missing from @microsoft/webui-examples-theme`);
  }

  const missing = requiredTokens.filter(
    (token) => !Object.prototype.hasOwnProperty.call(theme, token),
  );
  if (missing.length > 0) {
    console.warn(
      `Theme '${themeName}' is missing ${missing.length} protocol token(s): ${missing.join(", ")}`,
    );
  }

  return requiredTokens
    .filter((token) => Object.prototype.hasOwnProperty.call(theme, token))
    .sort((left, right) => left.localeCompare(right))
    .map((token) => `      --${token}: ${theme[token]};`)
    .join("\n");
}

function buildThemeStylesheet(lightCss: string, darkCss: string): string {
  return `:root {
  color-scheme: light dark;
${lightCss}
}

@media (prefers-color-scheme: dark) {
  :root {
    color-scheme: dark;
${darkCss}
  }
}
`;
}
