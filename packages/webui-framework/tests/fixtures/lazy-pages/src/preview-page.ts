// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '@microsoft/webui-framework';

export class PreviewPage extends WebUIElement {
  @observable help = '';
  @observable sibling = false;
  @observable nested = false;
  @observable warm = false;
  @observable message = 'Page-owned value';
  @observable count = 0;
  previewNode?: HTMLDivElement;
  preview(): void { this.help = 'hello'; }
  close(): void { this.help = ''; }
  showSibling(): void { this.sibling = true; }
  showNested(): void { this.nested = true; }
  increment(): void { this.count++; }
}
PreviewPage.define('preview-page');
