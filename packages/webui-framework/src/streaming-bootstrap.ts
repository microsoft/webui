// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { registerTemplateData } from './template-registry.js';
import { registerComponentStyles } from './element/style-catalog.js';
import { MAX_MARKER_SCAN_NODES, nextWithinRoot } from './streaming-dom.js';
import type { HydrationRange } from './streaming-dom.js';
import type {
  BoundaryBootstrap,
  SpanCompletionPayload,
  StreamedComponentStyles,
} from './streaming-protocol.js';

/** Register template data and merge response-scoped checkpoint metadata. */
export function applyBoundaryBootstrap(
  bootstrap: BoundaryBootstrap | SpanCompletionPayload,
  range?: HydrationRange,
): void {
  const buildId = bootstrap.buildId;
  const expected = window.__webui?.buildId;
  if (buildId !== undefined && (
    typeof buildId !== 'string' || buildId.length === 0 ||
    (expected !== undefined && expected !== buildId)
  )) {
    throw new Error('[WebUI] Resources use a different document build. Reload the document.');
  }
  const styles = bootstrap.componentStyles;
  if (styles?.strategy === 'style' && styles.resources) {
    resolveInlineStyles(styles.resources, range);
  }
  registerComponentStyles(styles);
  if (bootstrap.templates) registerTemplateData(bootstrap.templates);

  const runtime = window.__webui ??= {};
  const keys = Object.keys(bootstrap);
  for (let i = 0; i < keys.length; i++) {
    const key = keys[i];
    // Templates are merged by registerTemplateData; boundary state remains
    // ephemeral and is handed directly to roots by the activation walk.
    if (
      key === 'templates' ||
      key === 'state' ||
      key === 'stateRef' ||
      key === 'stateDelta' ||
      key === 'componentStyles' ||
      key === 'declarationId' ||
      key === 'enclosingSpanInstanceId'
    ) continue;
    if (key === 'inventory' || key === 'styleInventory') {
      runtime[key] = mergeInventory(runtime[key], bootstrap[key]);
      continue;
    }
    if (key === 'css' || key === 'styles') {
      appendUniqueStrings(runtime, key, bootstrap[key]);
      continue;
    }
    runtime[key] = bootstrap[key];
  }
}

// Read each promised source only once, before template registration can
// activate arbitrary roots. Stop at the last source or the range end.
function resolveInlineStyles(
  resources: StreamedComponentStyles['resources'],
  range: HydrationRange | undefined,
): void {
  let missing = 0;
  const ids = Object.keys(resources);
  for (const id of ids) {
    const resource = resources[id];
    if (resource?.kind === 'style' && resource.css === null) missing++;
  }
  if (!missing) return;

  const root = range?.start?.parentNode;
  const end = range?.end;
  if (root && end) {
    let node: Node | null | undefined = range.start?.nextSibling;
    for (let visited = 0; node && node !== end && visited < MAX_MARKER_SCAN_NODES; visited++) {
      if (node.nodeName === 'STYLE') {
        const source = node as HTMLStyleElement;
        if (source.hasAttribute('data-ws-style')) {
          const resource = resources[source.getAttribute('data-webui-resource') ?? ''];
          if (resource?.kind === 'style' && resource.css === null) {
            const css = source.textContent;
            if (css) {
              resource.css = css;
              source.removeAttribute('data-ws-style');
              if (--missing === 0) return;
            }
          }
        }
      }
      node = nextWithinRoot(node, root);
    }
    if (node && node !== end) {
      throw new Error(`[WebUI] Inline component stylesheet scan exceeded ${MAX_MARKER_SCAN_NODES} nodes in streaming checkpoint`);
    }
  }
  const unresolved = ids.filter(id => {
    const resource = resources[id];
    return resource?.kind === 'style' && resource.css === null;
  });
  throw new Error(`[WebUI] Missing inline component stylesheet in streaming checkpoint: ${unresolved.join(', ')}`);
}

/**
 * OR two hexadecimal inventory bitsets digit by digit.
 *
 * Both operands come from the Rust checkpoint serializer, so the digits are
 * trusted. The `typeof` guard only recovers the static type that the
 * `[key: string]: unknown` index signature on `Window['__webui']` erases.
 */
function mergeInventory(existing: unknown, delta: string | undefined): string {
  const current = typeof existing === 'string' ? existing : '';
  const next = delta ?? '';
  const length = Math.max(current.length, next.length);
  let merged = '';
  for (let i = 0; i < length; i++) {
    const a = i < current.length ? parseInt(current[i], 16) : 0;
    const b = i < next.length ? parseInt(next[i], 16) : 0;
    merged += (a | b).toString(16);
  }
  return merged;
}

/** Append the entries of one checkpoint's CSS or style delta not seen yet. */
function appendUniqueStrings(
  target: NonNullable<Window['__webui']>,
  key: 'css' | 'styles',
  delta: string[] | undefined,
): void {
  if (!delta) return;
  const existing = target[key];
  const cumulative = Array.isArray(existing) ? (existing as string[]) : [];
  for (let i = 0; i < delta.length; i++) {
    if (cumulative.indexOf(delta[i]) === -1) {
      cumulative.push(delta[i]);
    }
  }
  if (cumulative !== existing) target[key] = cumulative;
}
