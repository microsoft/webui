// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import '../../../src/lazy-hydration-entry.js';
import { WebUIElement, attr, observable } from '../../../src/index.js';

class CompletionRoot extends WebUIElement {
  @observable value = '0';
  @observable rows: Array<{ id: string; value: string }> = [];
  @observable showUnloaded = false;
  @observable payload = { label: 'loaded' };
  @observable html = '';
  @observable showAction = false;
  declare actionButton: HTMLButtonElement;
  captureInChanged = false;
  throwInChanged = false;
  pending: Promise<void> | undefined;

  async revealAction(): Promise<void> {
    this.showAction = true;
    await this.$whenRendered();
    this.actionButton.focus();
  }

  valueChanged(): void {
    if (this.captureInChanged) this.pending = this.$whenRendered();
    if (this.throwInChanged) throw new Error('test callback failed');
  }
}

class CompletionChild extends WebUIElement {
  @attr value = '0';
  @observable derived = 1;
  captureInChanged = false;
  pending: Promise<void> | undefined;

  valueChanged(_old: string | undefined, next: string): void {
    if (this.captureInChanged) this.pending = this.$whenRendered();
    this.derived = Number(next) + 1;
  }
}

class CompletionLeaf extends WebUIElement {
  @attr value = '0';
}

class CompletionLazy extends WebUIElement {
  @observable note = '';
}

class CompletionUnloaded extends WebUIElement {
  @observable payload = { label: '' };
}

declare global {
  interface Window {
    __defineCompletionUnloaded?: () => void;
  }
}

window.__webui!.templateHostExclusions ??= new Set();
window.__webui!.templateHostExclusions.add('test-completion-unloaded');
window.__defineCompletionUnloaded = () => CompletionUnloaded.define('test-completion-unloaded');

CompletionRoot.define('test-completion-root');
CompletionChild.define('test-completion-child');
CompletionLeaf.define('test-completion-leaf');
CompletionLazy.define('test-completion-lazy');
