// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

const injectedAssetStyles = new Set<string>();
let assetStylesSeeded = false;

interface WebUIAssetGlobal {
  nonce?: string;
  styles?: string[];
  [key: string]: unknown;
}

/** Parsed import maps that can be committed without further validation. */
export type PreparedAssetStyles = Record<string, string>[];

/** Validate and parse all import maps without mutating browser state. */
export function prepareAssetStyles(templateStyles: readonly string[]): PreparedAssetStyles {
  const prepared = new Array<Record<string, string>>(templateStyles.length);
  for (let i = 0; i < templateStyles.length; i++) {
    prepared[i] = parseImportMap(templateStyles[i]);
  }
  return prepared;
}

/** Read the configured CSP nonce used by component asset resources. */
export function readNonce(): string {
  const nonce = assetGlobal()?.nonce;
  if (nonce) return nonce;
  const meta = document.querySelector('meta[name="webui-nonce"]') as HTMLMetaElement | null;
  return meta?.content ?? '';
}

/** Commit import maps that were parsed during graph prevalidation. */
export function registerAssetStyles(
  preparedStyles: readonly Record<string, string>[],
  nonce: string,
): void {
  if (preparedStyles.length === 0) return;
  seedAssetStyleSet();

  for (let i = 0; i < preparedStyles.length; i++) {
    const imports = preparedStyles[i];
    const nextImports: Record<string, string> = {};
    let hasNewImport = false;
    const specifiers = Object.keys(imports);
    for (let j = 0; j < specifiers.length; j++) {
      const specifier = specifiers[j];
      if (injectedAssetStyles.has(specifier)) continue;
      injectedAssetStyles.add(specifier);
      nextImports[specifier] = imports[specifier];
      hasNewImport = true;
    }
    if (!hasNewImport) continue;

    const script = document.createElement('script');
    script.type = 'importmap';
    if (nonce) script.nonce = nonce;
    script.textContent = JSON.stringify({ imports: nextImports });
    document.head.appendChild(script);
  }
}

function assetGlobal(): WebUIAssetGlobal | undefined {
  return window.__webui as WebUIAssetGlobal | undefined;
}

function seedAssetStyleSet(): void {
  if (assetStylesSeeded) return;
  assetStylesSeeded = true;
  const styles = assetGlobal()?.styles;
  if (!styles) return;
  for (let i = 0; i < styles.length; i++) {
    injectedAssetStyles.add(styles[i]);
  }
}

function parseImportMap(scriptMarkup: string): Record<string, string> {
  const trimmed = scriptMarkup.trim();
  if (!trimmed.startsWith('<script')) {
    throw new Error('[WebUI] Component asset templateStyles entry must be a <script type="importmap"> tag.');
  }
  const openTagEnd = trimmed.indexOf('>');
  const closeTagStart = trimmed.lastIndexOf('</script>');
  if (openTagEnd < 0 || closeTagStart <= openTagEnd) {
    throw new Error('[WebUI] Component asset importmap tag is malformed.');
  }

  const parsed = JSON.parse(trimmed.substring(openTagEnd + 1, closeTagStart)) as {
    imports?: Record<string, unknown>;
  };
  if (!parsed.imports || typeof parsed.imports !== 'object') {
    throw new Error('[WebUI] Component asset importmap is missing an imports object.');
  }

  const imports: Record<string, string> = {};
  const specifiers = Object.keys(parsed.imports);
  for (let i = 0; i < specifiers.length; i++) {
    const specifier = specifiers[i];
    const uri = parsed.imports[specifier];
    if (typeof uri !== 'string' || !uri.startsWith('data:text/css,')) {
      throw new Error(`[WebUI] Component asset importmap entry "${specifier}" must be a data:text/css URI.`);
    }
    imports[specifier] = uri;
  }
  return imports;
}
