// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

export class TestDialog extends WebUIElement {
  @observable message = 'Hello from dialog';

  onClose(): void {
    this.$emit('close-dialog');
  }
}
TestDialog.define('test-dialog');
