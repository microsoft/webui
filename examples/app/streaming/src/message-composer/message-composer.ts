// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

/**
 * The composer is the highest-priority island on the page: it must paint
 * and become interactive before `DOMContentLoaded`, while the response
 * remains open (see DESIGN.md, "Progressive Streaming Hydration"). Its state
 * is purely client-local - there is no server round trip for typing or
 * posting - so hydration timing depends only on the composer boundary's
 * checkpoint, not on any later network activity.
 */
export class MessageComposer extends WebUIElement {
  @attr draft = '';
  @attr posted = '';

  onInput(e: Event): void {
    const target = e.target;
    if (target instanceof HTMLInputElement) {
      this.draft = target.value;
    }
  }

  onSubmit(e: SubmitEvent): void {
    e.preventDefault();
    const message = this.draft.trim();
    if (!message) return;
    this.posted = message;
    this.draft = '';
  }
}

MessageComposer.define('message-composer');
