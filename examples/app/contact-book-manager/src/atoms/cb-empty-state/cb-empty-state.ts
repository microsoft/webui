// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbEmptyState extends RenderableFASTElement(FASTElement) {
  @attr icon = '📭';
  @attr title = '';
  @attr message = '';
}

CbEmptyState.defineAsync({
  name: 'cb-empty-state',
  templateOptions: 'defer-and-hydrate',
});
