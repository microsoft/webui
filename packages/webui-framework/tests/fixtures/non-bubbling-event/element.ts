// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

interface Row {
  id: string;
}

/**
 * Regression coverage for event bindings a render-root listener cannot serve:
 * events that do not bubble, app-defined events the framework cannot enumerate,
 * and events stopped by an intermediate node. `@click` is the bubbling control.
 */
export class TestNonBubbling extends WebUIElement {
  @observable clicks = 0;
  @observable focuses = 0;
  @observable blurs = 0;
  @observable errors = 0;
  @observable enters = 0;
  @observable picks = 0;
  @observable cues = 0;
  @observable guardedClicks = 0;
  @observable lastRowId = '';
  @observable lastCurrentTarget = '';
  @observable items: Row[] = [{ id: 'a' }, { id: 'b' }];

  onClick(): void {
    this.clicks += 1;
  }

  onFocus(event: Event): void {
    this.focuses += 1;
    this.lastCurrentTarget = (event.currentTarget as Element | null)?.className ?? 'none';
  }

  onBlur(): void {
    this.blurs += 1;
  }

  onImgError(): void {
    this.errors += 1;
  }

  onEnter(): void {
    this.enters += 1;
  }

  onPicked(): void {
    this.picks += 1;
  }

  onCueChange(): void {
    this.cues += 1;
  }

  onGuarded(): void {
    this.guardedClicks += 1;
  }

  onRowFocus(id: string, event: Event): void {
    const attr = (event.currentTarget as Element | null)?.getAttribute('data-id') ?? 'none';
    this.lastRowId = `${id}:${attr}`;
  }
}

TestNonBubbling.define('test-non-bubbling');
