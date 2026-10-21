// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  injectCssLinks,
  registerTemplatesAndStyles,
  waitForTemplateReadiness,
} from './templates.js';
import type { StreamingPartialResponse } from './partial-response.js';

/** Resource publication/readiness shared by non-streaming partial consumers. */
export interface PartialResourceContext {
  readonly nonce: string;
  readonly injectedCss: Set<string>;
  updateInventory(inventory: string, styleInventory?: string): void;
}

/** Decode a JSON partial and prepare its resources without route reconciliation. */
export async function readJsonPartial(
  response: Response,
  context: PartialResourceContext,
  signal?: AbortSignal,
): Promise<StreamingPartialResponse | null> {
  const data: unknown = await response.json();
  if (signal?.aborted) return null;
  if (!data || typeof data !== 'object' || Array.isArray(data)) {
    throw new Error('[WebUI] A partial response must be a JSON object.');
  }
  const partial = data as StreamingPartialResponse;
  const ready = registerTemplatesAndStyles(
    partial, context.nonce, context.injectedCss, (inventory, styles) => context.updateInventory(inventory, styles),
  );
  injectCssLinks(partial, context.injectedCss);
  if (ready && !await waitForTemplateReadiness(ready, signal)) return null;
  return signal?.aborted ? null : partial;
}
