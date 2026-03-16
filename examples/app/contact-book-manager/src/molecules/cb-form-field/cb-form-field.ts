// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbFormField extends RenderableFASTElement(FASTElement) {
  @attr label = '';
  @attr name = '';
  @attr value = '';
  @attr placeholder = '';
  @attr type = 'text';
  @attr error = '';
}

CbFormField.defineAsync({
  name: 'cb-form-field',
  templateOptions: 'defer-and-hydrate',
});
