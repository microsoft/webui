// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { attributeMap } from '@microsoft/fast-element/attribute-map.js';
import { declarativeTemplate } from '@microsoft/fast-element/declarative.js';
import { observerMap } from '@microsoft/fast-element/observer-map.js';
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

export class CalcApp extends FASTElement {
  @attr mode!: string;
  @attr({ attribute: 'display-value' }) displayValue!: string;
  @attr expression!: string;
  @attr({ attribute: 'columns' }) gridColumns!: string;

  @observable buttons!: ButtonData[];

  private state!: CalcState;

  private prepareFromDom(): void {
    this.mode = this.getAttribute('mode') || 'standard';
    this.displayValue = this.getAttribute('display-value') || '0';
    this.expression = this.getAttribute('expression') || '';
    this.gridColumns = this.getAttribute('columns') || '4';
    this.state = createInitialState();

    // Read button data from pre-rendered DOM (mirrors todo-fast pattern)
    const buttons: ButtonData[] = [];
    if (this.shadowRoot) {
      for (const el of this.shadowRoot.querySelectorAll('calc-button')) {
        buttons.push({
          label: el.getAttribute('label') || '',
          value: el.getAttribute('value') || '',
          type: el.getAttribute('btn-type') || '',
          span: el.getAttribute('btn-span') || '1',
        });
      }
    }

    if (buttons.length > 0) {
      this.buttons = buttons;
    } else {
      this.loadButtonsFromEngine();
    }
  }

  private boundKeydown = this.onKeydown.bind(this);

  connectedCallback(): void {
    this.prepareFromDom();
    super.connectedCallback();
    void this.$fastController.isPrerendered.then(() => {
      this.prepareFromDom();
      this.updateActiveModeTab();
    });
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

    this.gridColumns = String(engine.columns);
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

void CalcApp.define({
  name: 'calc-app',
  template: declarativeTemplate(),
}, [attributeMap(), observerMap()]);
