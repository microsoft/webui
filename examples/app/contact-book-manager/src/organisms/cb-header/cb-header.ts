// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

type WindowAction = 'minimize' | 'toggle-maximize' | 'close';
type WindowHost = Window & {
  webuiHostPostMessage?: (payload: string) => void;
};

export class CbHeader extends WebUIElement {
  @attr mode: 'desktop' | 'web' = 'web';
  @attr searchQuery = '';

  onWindowAction(action: WindowAction): void {
    if (this.mode !== 'desktop') return;
    (window as WindowHost).webuiHostPostMessage?.(JSON.stringify(action));
  }

  onInput(e: Event): void {
    const input = e.currentTarget;
    if (!(input instanceof HTMLInputElement)) return;

    this.searchQuery = input.value;
    this.$emit('search-change', { value: input.value });
  }
}

CbHeader.define('cb-header');
