// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TemplateBlockMeta, TemplateMeta } from './template-types.js';

let resolveFragment: ((id: string, owner: string) => TemplateMeta | undefined) | undefined;
let ownerOfBlock: ((meta: TemplateBlockMeta) => TemplateMeta | undefined) | undefined;

export function installDeferredResolver(
  resolve: NonNullable<typeof resolveFragment>,
  owner: NonNullable<typeof ownerOfBlock>,
): void {
  resolveFragment = resolve;
  ownerOfBlock = owner;
}

export function deferredBlockOwner(meta: TemplateBlockMeta): TemplateMeta | undefined {
  return ownerOfBlock?.(meta);
}

export function findDeferredFragment(id: string, owner: string): TemplateMeta | undefined {
  return resolveFragment?.(id, owner);
}

export function requireDeferredFragment(id: string, owner: string): TemplateMeta {
  if (!resolveFragment) {
    throw new Error(
      '[WebUI] Import @microsoft/webui-framework/deferred.js before activating deferred UI.',
    );
  }
  const template = resolveFragment(id, owner);
  if (!template) {
    throw new Error(
      `[WebUI] Fragment "${id}" is not loaded. Await resources before activation.`,
    );
  }
  return template;
}
