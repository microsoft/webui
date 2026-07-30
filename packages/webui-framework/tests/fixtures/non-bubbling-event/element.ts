// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

interface Row {
  id: string;
}

/**
 * Exercises event bindings whose events do **not** bubble.  Delegation on the
 * render root cannot see these, so the framework must wire them as direct
 * listeners on the bound element.  `@click` is included as the bubbling
 * control that must keep using delegation.
 */
export class TestNonBubbling extends WebUIElement {
  @observable clicks = 0;
  @observable focuses = 0;
  @observable blurs = 0;
  @observable errors = 0;
  @observable enters = 0;
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

  onRowFocus(id: string, event: Event): void {
    const attr = (event.currentTarget as Element | null)?.getAttribute('data-id') ?? 'none';
    this.lastRowId = `${id}:${attr}`;
  }
}

TestNonBubbling.define('test-non-bubbling');
