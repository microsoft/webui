// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MpPageAbout extends RenderableFASTElement(FASTElement) {}

MpPageAbout.defineAsync({
  name: 'mp-page-about',
  templateOptions: 'defer-and-hydrate',
});
