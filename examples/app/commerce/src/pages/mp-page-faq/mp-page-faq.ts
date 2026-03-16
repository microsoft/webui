// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MpPageFaq extends RenderableFASTElement(FASTElement) {}

MpPageFaq.defineAsync({
  name: 'mp-page-faq',
  templateOptions: 'defer-and-hydrate',
});
