// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

interface Message {
  messageId: string;
  kind: string;
  hidden: boolean;
}

interface Turn {
  turnId: string;
  messages: Message[];
  terminal: { type: string };
}

interface MinimalTurn {
  messages: string[];
  failed: boolean;
}

function terminalTurn(turnId: string): Turn {
  return {
    turnId,
    messages: [
      { messageId: `${turnId}-user`, kind: 'user', hidden: false },
      { messageId: `${turnId}-assistant`, kind: 'assistant', hidden: false },
    ],
    terminal: { type: 'failed' },
  };
}

export class TestStructuralSiblingOrder extends WebUIElement {
  @observable turns: Turn[] = [];
  @observable minimalTurns: MinimalTurn[] = [];
  @observable beforeRepeat = '';
  @observable beforeBlocks = '';
  @observable betweenBlocks = '';
  @observable showTextConditional = true;
  @observable textItems: string[] = [];

  appendExactTerminal(): void {
    this.turns = [...this.turns, terminalTurn('live-exact')];
  }

  appendMinimalTerminal(): void {
    this.minimalTurns = [
      ...this.minimalTurns,
      { messages: ['user', 'assistant'], failed: true },
    ];
  }

  setBeforeRepeat(): void {
    this.beforeRepeat = 'before';
  }

  setBlockTexts(): void {
    this.beforeBlocks = 'before';
    this.betweenBlocks = 'between';
  }
}

TestStructuralSiblingOrder.define('test-structural-sibling-order');
