// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { ModeEngine, ButtonDef, CalcState } from './engine.js';
import { registerMode } from './engine.js';

const buttons: ButtonDef[] = [
  // Row 1 — Scientific functions
  { label: 'sin', value: 'sin', type: 'function' },
  { label: 'cos', value: 'cos', type: 'function' },
  { label: 'tan', value: 'tan', type: 'function' },
  { label: '(', value: '(', type: 'function' },
  { label: ')', value: ')', type: 'function' },

  // Row 2 — More functions
  { label: 'ln', value: 'ln', type: 'function' },
  { label: 'log', value: 'log', type: 'function' },
  { label: '√', value: 'sqrt', type: 'function' },
  { label: 'x²', value: 'square', type: 'function' },
  { label: 'xʸ', value: 'power', type: 'function' },

  // Row 3 — Constants & extras
  { label: 'π', value: 'pi', type: 'function' },
  { label: 'e', value: 'euler', type: 'function' },
  { label: 'x!', value: 'factorial', type: 'function' },
  { label: '±', value: 'negate', type: 'action' },
  { label: '%', value: 'percent', type: 'action' },

  // Row 4 — Standard row
  { label: 'AC', value: 'clear', type: 'action' },
  { label: '7', value: '7', type: 'number' },
  { label: '8', value: '8', type: 'number' },
  { label: '9', value: '9', type: 'number' },
  { label: '÷', value: '/', type: 'operator' },

  // Row 5
  { label: 'MC', value: 'mc', type: 'action' },
  { label: '4', value: '4', type: 'number' },
  { label: '5', value: '5', type: 'number' },
  { label: '6', value: '6', type: 'number' },
  { label: '×', value: '*', type: 'operator' },

  // Row 6
  { label: 'MR', value: 'mr', type: 'action' },
  { label: '1', value: '1', type: 'number' },
  { label: '2', value: '2', type: 'number' },
  { label: '3', value: '3', type: 'number' },
  { label: '−', value: '-', type: 'operator' },

  // Row 7
  { label: 'M+', value: 'm+', type: 'action' },
  { label: '0', value: '0', type: 'number', span: 2 },
  { label: '.', value: '.', type: 'number' },
  { label: '+', value: '+', type: 'operator' },

  // Row 8 — Equals
  { label: '=', value: '=', type: 'equal', span: 5 },
];

function factorial(n: number): number {
  if (n < 0 || !Number.isInteger(n)) return NaN;
  if (n > 170) return Infinity;
  let result = 1;
  for (let i = 2; i <= n; i++) {
    result *= i;
  }
  return result;
}

function evaluate(expression: string): number {
  // Tokenize into numbers, operators, and parentheses
  const tokens: (number | string)[] = [];
  let current = '';

  for (let i = 0; i < expression.length; i++) {
    const ch = expression[i];
    if (ch === ' ') continue;

    if (ch === '(' || ch === ')') {
      if (current !== '') {
        tokens.push(parseFloat(current));
        current = '';
      }
      tokens.push(ch);
    } else if (ch === '+' || ch === '×' || ch === '÷') {
      if (current !== '') {
        tokens.push(parseFloat(current));
        current = '';
      }
      tokens.push(ch);
    } else if (ch === '−') {
      if (current === '' && (tokens.length === 0 || typeof tokens[tokens.length - 1] === 'string' && tokens[tokens.length - 1] !== ')')) {
        current += '-';
      } else {
        if (current !== '') {
          tokens.push(parseFloat(current));
          current = '';
        }
        tokens.push(ch);
      }
    } else {
      current += ch;
    }
  }
  if (current !== '') {
    tokens.push(parseFloat(current));
  }

  return evalTokens(tokens);
}

function evalTokens(tokens: (number | string)[]): number {
  // Handle parentheses iteratively using a stack
  const stack: (number | string)[][] = [[]];

  for (const token of tokens) {
    if (token === '(') {
      stack.push([]);
    } else if (token === ')') {
      const inner = stack.pop();
      if (!inner || stack.length === 0) return NaN;
      const val = evalFlat(inner);
      stack[stack.length - 1].push(val);
    } else {
      stack[stack.length - 1].push(token);
    }
  }

  return evalFlat(stack[0]);
}

function evalFlat(tokens: (number | string)[]): number {
  if (tokens.length === 0) return 0;

  // Multiply and divide first
  const addSub: (number | string)[] = [];
  let i = 0;
  while (i < tokens.length) {
    if (typeof tokens[i] === 'string' && (tokens[i] === '×' || tokens[i] === '÷')) {
      const left = addSub.pop() as number;
      const right = tokens[i + 1] as number;
      if (tokens[i] === '×') {
        addSub.push(left * right);
      } else {
        addSub.push(right === 0 ? NaN : left / right);
      }
      i += 2;
    } else {
      addSub.push(tokens[i]);
      i++;
    }
  }

  // Add and subtract
  let result = addSub[0] as number;
  for (let j = 1; j < addSub.length; j += 2) {
    const op = addSub[j] as string;
    const val = addSub[j + 1] as number;
    if (op === '+') {
      result += val;
    } else if (op === '−') {
      result -= val;
    }
  }

  return result;
}

function formatNumber(n: number): string {
  if (!isFinite(n)) return 'Error';
  if (Number.isNaN(n)) return 'Error';

  const str = String(n);
  if (str.length > 12) {
    const precise = n.toPrecision(10);
    if (precise.includes('.')) {
      return precise.replace(/\.?0+$/, '');
    }
    return precise;
  }
  return str;
}

const OP_DISPLAY: Record<string, string> = {
  '/': ' ÷ ',
  '*': ' × ',
  '-': ' − ',
  '+': ' + ',
};

class ScientificEngine implements ModeEngine {
  readonly name = 'Scientific';
  readonly columns = 5;
  readonly buttons = buttons;

  private pendingOp: string | null = null;
  private leftOperand: number | null = null;
  private expressionParts: string[] = [];
  private parenDepth = 0;

  processInput(input: string, state: CalcState): CalcState {
    const next = { ...state, error: null };

    // Number input
    if (/^[0-9]$/.test(input)) {
      if (next.resetOnNext || next.display === '0') {
        next.display = input;
        next.resetOnNext = false;
      } else if (next.display.replace(/[^0-9]/g, '').length < 15) {
        next.display += input;
      }
      return next;
    }

    // Decimal
    if (input === '.') {
      if (next.resetOnNext) {
        next.display = '0.';
        next.resetOnNext = false;
      } else if (!next.display.includes('.')) {
        next.display += '.';
      }
      return next;
    }

    // Clear
    if (input === 'clear') {
      next.display = '0';
      next.expression = '';
      next.resetOnNext = false;
      next.error = null;
      this.pendingOp = null;
      this.leftOperand = null;
      this.expressionParts = [];
      this.parenDepth = 0;
      return next;
    }

    // Negate
    if (input === 'negate') {
      if (next.display !== '0' && next.display !== 'Error') {
        next.display = next.display.startsWith('-')
          ? next.display.slice(1)
          : '-' + next.display;
      }
      return next;
    }

    // Percent
    if (input === 'percent') {
      const val = parseFloat(next.display);
      if (!isNaN(val)) {
        next.display = formatNumber(val / 100);
      }
      return next;
    }

    // Constants
    if (input === 'pi') {
      next.display = formatNumber(Math.PI);
      next.resetOnNext = false;
      return next;
    }
    if (input === 'euler') {
      next.display = formatNumber(Math.E);
      next.resetOnNext = false;
      return next;
    }

    // Unary functions
    const unaryFns: Record<string, (x: number) => number> = {
      sin: (x) => Math.sin(x * Math.PI / 180),
      cos: (x) => Math.cos(x * Math.PI / 180),
      tan: (x) => Math.tan(x * Math.PI / 180),
      ln: (x) => Math.log(x),
      log: (x) => Math.log10(x),
      sqrt: (x) => Math.sqrt(x),
      square: (x) => x * x,
      factorial: (x) => factorial(x),
    };

    if (input in unaryFns) {
      const val = parseFloat(next.display);
      if (!isNaN(val)) {
        const result = unaryFns[input](val);
        const formatted = formatNumber(result);
        const fnLabel = input === 'square' ? '²' : input === 'factorial' ? '!' : input;

        if (input === 'square') {
          next.expression = `(${next.display})² =`;
        } else if (input === 'factorial') {
          next.expression = `${next.display}! =`;
        } else {
          next.expression = `${fnLabel}(${next.display}) =`;
        }

        next.display = formatted;
        if (formatted === 'Error') {
          next.error = 'Invalid operation';
        }
        next.resetOnNext = true;
      }
      return next;
    }

    // Power (xʸ) — acts like an operator
    if (input === 'power') {
      const currentVal = parseFloat(next.display);
      this.leftOperand = currentVal;
      this.pendingOp = 'power';
      this.expressionParts = [next.display, ' ^ '];
      next.expression = this.expressionParts.join('');
      next.resetOnNext = true;
      return next;
    }

    // Parentheses
    if (input === '(') {
      this.parenDepth++;
      this.expressionParts.push('(');
      next.expression = this.expressionParts.join('');
      next.resetOnNext = true;
      return next;
    }
    if (input === ')') {
      if (this.parenDepth > 0) {
        this.parenDepth--;
        this.expressionParts.push(next.display);
        this.expressionParts.push(')');
        next.expression = this.expressionParts.join('');
        next.resetOnNext = true;
      }
      return next;
    }

    // Memory operations
    if (input === 'mc') {
      next.memory = 0;
      return next;
    }
    if (input === 'mr') {
      next.display = formatNumber(next.memory);
      next.resetOnNext = true;
      return next;
    }
    if (input === 'm+') {
      const val = parseFloat(next.display);
      if (!isNaN(val)) {
        next.memory += val;
      }
      return next;
    }

    // Operators
    if (['+', '-', '*', '/'].includes(input)) {
      const currentVal = parseFloat(next.display);

      if (this.pendingOp === 'power' && this.leftOperand !== null && !next.resetOnNext) {
        const result = Math.pow(this.leftOperand, currentVal);
        next.display = formatNumber(result);
        this.expressionParts = [formatNumber(result)];
        this.leftOperand = result;
      } else if (this.pendingOp !== null && this.pendingOp !== 'power' && this.leftOperand !== null && !next.resetOnNext) {
        const exprStr = this.expressionParts.join('') + next.display;
        const result = evaluate(exprStr);
        next.display = formatNumber(result);
        this.expressionParts = [formatNumber(result)];
        this.leftOperand = result;
      } else {
        this.leftOperand = currentVal;
        if (this.expressionParts.length === 0 || next.resetOnNext) {
          this.expressionParts = [next.display];
        }
      }

      this.pendingOp = input;
      this.expressionParts.push(OP_DISPLAY[input]);
      next.expression = this.expressionParts.join('');
      next.resetOnNext = true;
      return next;
    }

    // Equals
    if (input === '=') {
      if (this.pendingOp !== null && this.leftOperand !== null) {
        let result: number;
        if (this.pendingOp === 'power') {
          result = Math.pow(this.leftOperand, parseFloat(next.display));
          this.expressionParts.push(next.display);
        } else {
          this.expressionParts.push(next.display);
          const exprStr = this.expressionParts.join('');
          result = evaluate(exprStr);
        }

        const displayResult = formatNumber(result);
        next.expression = this.expressionParts.join('') + ' =';
        next.display = displayResult;

        if (displayResult === 'Error') {
          next.error = 'Invalid operation';
        }

        this.pendingOp = null;
        this.leftOperand = null;
        this.expressionParts = [];
        next.resetOnNext = true;
      }
      return next;
    }

    return next;
  }
}

const engine = new ScientificEngine();
registerMode('scientific', engine);

export { engine as scientificEngine };
