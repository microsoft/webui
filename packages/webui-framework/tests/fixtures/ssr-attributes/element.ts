// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '../../../src/index.js';

class TestAttributeDialog extends WebUIElement {
  @attr({ mode: 'boolean' }) open = false;
  @attr({ attribute: 'aria-label' }) ariaLabel = 'Default name';
  @attr({ attribute: 'aria-describedby' }) ariaDescribedby = '';
  @attr({ attribute: 'dialog-title' }) heading = 'Default title';
  dialogEl: HTMLDialogElement | null = null;

  hydratedCallback(): void {
    if (this.id === 'literal' && this.open && this.dialogEl) {
      this.dialogEl.close();
      this.dialogEl.showModal();
    }
  }
}

TestAttributeDialog.define('test-attribute-dialog');

class InheritedDialogBase extends WebUIElement {
  @attr({ attribute: 'old-expanded', mode: 'boolean' }) expanded: boolean | string = false;
}

class TestInheritedDialog extends InheritedDialogBase {
  @attr({ attribute: 'new-expanded' }) override expanded: boolean | string = false;
}

TestInheritedDialog.define('test-inherited-dialog');

class TestStackedDialog extends WebUIElement {
  @attr({ attribute: 'first-expanded', mode: 'boolean' })
  @attr({ attribute: 'second-expanded' })
  expanded: boolean | string = false;
}

TestStackedDialog.define('test-stacked-dialog');
