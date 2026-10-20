// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { CompiledSSRLayout, TemplateBlockMeta } from './template-types.js';
import { validateTrustedTemplateBlock } from './trusted-types-policy.js';

const validatedLayouts = new WeakSet<TemplateBlockMeta>();

/** Require compiler-owned topology before adopting an SSR subtree. */
export function getTemplateSSRLayout(meta: TemplateBlockMeta): CompiledSSRLayout {
  const layout = meta.s;
  if (layout === undefined) throw invalidLayout();
  if (!validatedLayouts.has(meta)) {
    validateLayout(meta);
    validateTrustedTemplateBlock(meta);
    validatedLayouts.add(meta);
  }
  return layout;
}

function validateLayout(meta: TemplateBlockMeta): void {
  const layout = meta.s;
  if (!Array.isArray(layout) || layout.length < 2 || !validInteger(layout[0])) {
    throw invalidLayout();
  }
  let remaining = 1;
  for (let i = 1; i < layout.length; i++) {
    const shape = layout[i];
    if (!validInteger(shape) || remaining === 0) throw invalidLayout();
    remaining += (shape >> 1) - 1;
  }
  if (remaining !== 0 || layout[0] < (layout[1] >> 1)) throw invalidLayout();
  if (meta.tx) {
    for (let i = 0; i < meta.tx.length; i++) {
      const ref = meta.tx[i][3];
      if (ref !== undefined && (!validInteger(ref) || (ref & 3) === 0)) throw invalidLayout();
    }
  }
}

function validInteger(value: number): boolean {
  return Number.isInteger(value) && value >= 0 && value <= 0x7fffffff;
}

function invalidLayout(): Error {
  return new Error('[WebUI] Invalid compiled SSR layout. Rebuild templates with a compatible WebUI compiler.');
}
