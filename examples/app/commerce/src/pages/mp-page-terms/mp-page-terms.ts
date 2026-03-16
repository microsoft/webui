// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MpPageTerms extends RenderableFASTElement(FASTElement) {}

MpPageTerms.defineAsync({
  name: 'mp-page-terms',
  templateOptions: 'defer-and-hydrate',
});
