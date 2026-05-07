// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';
import type { CalcState, ButtonDef } from '../modes/engine.js';
import { createInitialState, getMode } from '../modes/engine.js';
import '../modes/standard.js';
import '../modes/scientific.js';

interface ButtonData {
  label: string;
  value: string;
  type: string;
  span: string;
}

export class CalcApp extends WebUIElement {
  @attr mode = 'standard';
  @attr displayValue = '0';
  @attr expression = '';
  @attr columns = '4';

  @observable buttons: ButtonData[] = [];

  private state: CalcState = createInitialState();
  private boundKeydown = this.onKeydown.bind(this);

  connectedCallback(): void {
    super.connectedCallback();
    document.addEventListener('keydown', this.boundKeydown);
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    document.removeEventListener('keydown', this.boundKeydown);
  }

  onButtonPress(e: CustomEvent<{ value: string }>): void {
    this.handleInput(e.detail.value);
  }

  onModeSelect(e: MouseEvent): void {
    const target = e.composedPath()[0] as HTMLElement;
    const btn = target.closest('[data-mode]') as HTMLElement | null;
    if (!btn) return;

    const newMode = btn.getAttribute('data-mode');
    if (newMode && newMode !== this.mode) {
      this.mode = newMode;
      this.state = {
        ...this.state,
        expression: '',
        resetOnNext: true,
        error: null,
      };
      this.expression = '';
      this.loadButtonsFromEngine();
      this.updateActiveModeTab();
    }
  }

  private onKeydown(e: KeyboardEvent): void {
    const keyMap: Record<string, string> = {
      '0': '0', '1': '1', '2': '2', '3': '3', '4': '4',
      '5': '5', '6': '6', '7': '7', '8': '8', '9': '9',
      '.': '.', '+': '+', '-': '-', '*': '*', '/': '/',
      Enter: '=', '=': '=', Escape: 'clear', Backspace: 'clear',
      '%': 'percent',
    };

    const input = keyMap[e.key];
    if (input) {
      e.preventDefault();
      this.handleInput(input);
    }
  }

  private handleInput(input: string): void {
    const engine = getMode(this.mode);
    if (!engine) return;

    this.state = engine.processInput(input, this.state);
    this.displayValue = this.state.display;
    this.expression = this.state.expression;
  }

  private loadButtonsFromEngine(): void {
    const engine = getMode(this.mode);
    if (!engine) return;

    this.columns = String(engine.columns);
    this.buttons = engine.buttons.map((b: ButtonDef) => ({
      label: b.label,
      value: b.value,
      type: b.type,
      span: String(b.span ?? 1),
    }));
  }

  private updateActiveModeTab(): void {
    if (!this.shadowRoot) return;
    for (const tab of this.shadowRoot.querySelectorAll('[data-mode]')) {
      if (tab.getAttribute('data-mode') === this.mode) {
        tab.setAttribute('data-active', '');
      } else {
        tab.removeAttribute('data-active');
      }
    }
  }
}

CalcApp.define('calc-app');
