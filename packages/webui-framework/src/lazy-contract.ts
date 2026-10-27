// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TemplateBlockMeta, TemplateMeta } from './template-types.js';

// The rendering core only knows that a string block reference names a hostless
// template resolved here. `lazy.js` installs the resolver and extends the
// element with pack loading, triggers, and release, so pages without lazy
// regions ship no lazy code.
let resolveFragment: ((id: string, owner: string) => TemplateMeta | undefined) | undefined;
let ownerOfBlock: ((meta: TemplateBlockMeta) => TemplateMeta | undefined) | undefined;

export function installLazyResolver(
  resolve: NonNullable<typeof resolveFragment>,
  owner: NonNullable<typeof ownerOfBlock>,
): void {
  resolveFragment = resolve;
  ownerOfBlock = owner;
}

export function lazyBlockOwner(meta: TemplateBlockMeta): TemplateMeta | undefined {
  return ownerOfBlock?.(meta);
}

export function findLazyFragment(id: string, owner: string): TemplateMeta | undefined {
  return resolveFragment?.(id, owner);
}

export function requireLazyFragment(id: string, owner: string): TemplateMeta {
  if (!resolveFragment) {
    throw new Error(
      '[WebUI] Import @microsoft/webui-framework/lazy.js before activating lazy UI.',
    );
  }
  const template = resolveFragment(id, owner);
  if (!template) {
    throw new Error(
      `[WebUI] Fragment "${id}" is missing. Reload matching resources.`,
    );
  }
  return template;
}
