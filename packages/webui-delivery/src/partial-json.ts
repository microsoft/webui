// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  injectCssLinks,
  registerTemplatesAndStyles,
  waitForTemplateReadiness,
} from './templates.js';
import type { TemplateResourceResponse } from './templates.js';

/** Resource publication/readiness shared by non-streaming partial consumers. */
export interface PartialResourceContext {
  readonly nonce: string;
  readonly injectedCss: Set<string>;
  updateInventory(inventory: string, styleInventory?: string): void;
}

/** Decode a JSON partial and prepare its resources without route reconciliation. */
export async function readJsonPartial<T extends TemplateResourceResponse = TemplateResourceResponse>(
  response: Response,
  context: PartialResourceContext,
  signal?: AbortSignal,
): Promise<T | null> {
  const data: unknown = await response.json();
  return preparePartial<T>(data, context, signal);
}

/** Publish a decoded resource envelope without imposing navigation or state policy. */
export async function preparePartial<T extends TemplateResourceResponse = TemplateResourceResponse>(
  data: unknown,
  context: PartialResourceContext,
  signal?: AbortSignal,
): Promise<T | null> {
  if (signal?.aborted) return null;
  if (!data || typeof data !== 'object' || Array.isArray(data)) {
    throw new Error('[WebUI] A partial response must be a JSON object.');
  }
  const partial = data as T;
  const ready = registerTemplatesAndStyles(
    partial, context.nonce, (inventory, styles) => context.updateInventory(inventory, styles),
  );
  injectCssLinks(partial, context.injectedCss);
  if (ready && !await waitForTemplateReadiness(ready, signal)) return null;
  return signal?.aborted ? null : partial;
}
