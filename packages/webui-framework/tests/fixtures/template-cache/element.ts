// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { collectTemplates } from '../../../src/template-cache-entry.js';
import { getTemplate, observable, WebUIElement } from '../../../src/core.js';
import type { TemplateMeta } from '../../../src/template-types.js';

class CacheParent extends WebUIElement {
  @observable show = false;
  @observable message = 'Before collection';

  showChild(): void {
    this.message = 'Dependency survived collection';
    this.show = true;
  }
}

CacheParent.define('cache-parent');

function weakTemplate(tag: string): WeakRef<TemplateMeta> {
  const meta = getTemplate(tag);
  if (!meta) throw new Error(`Missing fixture template ${tag}`);
  return new WeakRef(meta);
}

export interface TemplateCacheFixture {
  unused: WeakRef<TemplateMeta>;
  collect(): number;
  initiallyReleased: number;
}

declare global {
  interface Window {
    __templateCacheFixture: TemplateCacheFixture;
  }
}

window.__templateCacheFixture = {
  unused: weakTemplate('cache-unused'),
  collect: collectTemplates,
  initiallyReleased: collectTemplates(),
};
