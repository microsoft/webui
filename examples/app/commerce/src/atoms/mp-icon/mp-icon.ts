// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MpIcon extends RenderableFASTElement(FASTElement) {
  @attr name = '';

  async prepare(): Promise<void> {
    this.name = this.getAttribute('name') || '';
  }
}

MpIcon.defineAsync({
  name: 'mp-icon',
  templateOptions: 'defer-and-hydrate',
});
