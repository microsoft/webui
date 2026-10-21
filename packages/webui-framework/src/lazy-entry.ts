// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { installLazyResolver } from './lazy-contract.js';
import { getTemplate } from './template-registry.js';
import type { TemplateBlockMeta, TemplateMeta } from './template-types.js';

const prepared = new WeakSet<TemplateMeta>();
const owners = new WeakMap<TemplateBlockMeta, TemplateMeta>();
installLazyResolver((id, owner) => {
  const template = getTemplate(id);
  if (!template) return undefined;
  if (template.f !== id || template.o !== owner) {
    throw new Error(
      `[WebUI] Template fragment "${id}" does not belong to <${owner}>. Rebuild matching client and protocol artifacts.`,
    );
  }
  if (!prepared.has(template)) {
    prepared.add(template);
    const blocks = template.b;
    if (blocks) {
      for (let i = 0; i < blocks.length; i++) owners.set(blocks[i], template);
    }
  }
  return template;
}, meta => owners.get(meta));

if (typeof window !== 'undefined') {
  window.__webuiResolveResources = root => {
    const pending = [root];
    const seen = new Set<string>();
    const required: string[] = [];
    while (pending.length > 0) {
      const id = pending.pop()!;
      if (seen.has(id)) continue;
      seen.add(id);
      const template = getTemplate(id);
      if (!template) return undefined;
      required.push(id);
      const dependencies = template.d;
      if (dependencies) {
        for (let i = 0; i < dependencies.length; i++) pending.push(dependencies[i]);
      }
    }
    return required;
  };
}
