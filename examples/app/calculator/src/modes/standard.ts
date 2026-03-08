import type { ModeEngine, ButtonDef, CalcState } from './engine.js';
import { registerMode } from './engine.js';

const buttons: ButtonDef[] = [
  { label: 'AC', value: 'clear', type: 'action' },
  { label: '±', value: 'negate', type: 'action' },
  { label: '%', value: 'percent', type: 'action' },
  { label: '÷', value: '/', type: 'operator' },

  { label: '7', value: '7', type: 'number' },
  { label: '8', value: '8', type: 'number' },
  { label: '9', value: '9', type: 'number' },
  { label: '×', value: '*', type: 'operator' },

  { label: '4', value: '4', type: 'number' },
  { label: '5', value: '5', type: 'number' },
  { label: '6', value: '6', type: 'number' },
  { label: '−', value: '-', type: 'operator' },

  { label: '1', value: '1', type: 'number' },
  { label: '2', value: '2', type: 'number' },
  { label: '3', value: '3', type: 'number' },
  { label: '+', value: '+', type: 'operator' },

  { label: '0', value: '0', type: 'number', span: 2 },
  { label: '.', value: '.', type: 'number' },
  { label: '=', value: '=', type: 'equal' },
];

function evaluate(expression: string): number {
  // Tokenize the expression into numbers and operators
  const tokens: (number | string)[] = [];
  let current = '';

  for (let i = 0; i < expression.length; i++) {
    const ch = expression[i];
    if (ch === ' ') continue;

    if (ch === '+' || ch === '×' || ch === '÷') {
      if (current !== '') {
        tokens.push(parseFloat(current));
        current = '';
      }
      tokens.push(ch);
    } else if (ch === '−') {
      // Distinguish negative sign from subtraction
      if (current === '' && (tokens.length === 0 || typeof tokens[tokens.length - 1] === 'string')) {
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

  if (tokens.length === 0) return 0;

  // First pass: multiply and divide
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

  // Second pass: add and subtract
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

  // Avoid floating point display issues
  const str = String(n);
  if (str.length > 12) {
    // Use toPrecision for very large/small numbers
    const precise = n.toPrecision(10);
    // Remove trailing zeros after decimal
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

class StandardEngine implements ModeEngine {
  readonly name = 'Standard';
  readonly columns = 4;
  readonly buttons = buttons;

  private pendingOp: string | null = null;
  private leftOperand: number | null = null;
  private expressionParts: string[] = [];

  processInput(input: string, state: CalcState): CalcState {
    const next = { ...state, error: null };

    // Number input
    if (/^[0-9]$/.test(input)) {
      if (next.resetOnNext || next.display === '0') {
        next.display = input;
        next.resetOnNext = false;
      } else if (next.display.replace(/[^0-9]/g, '').length < 12) {
        next.display += input;
      }
      return next;
    }

    // Decimal point
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

    // Operators
    if (['+', '-', '*', '/'].includes(input)) {
      const currentVal = parseFloat(next.display);

      if (this.pendingOp !== null && this.leftOperand !== null && !next.resetOnNext) {
        // Chain: evaluate pending operation first
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
        this.expressionParts.push(next.display);
        const exprStr = this.expressionParts.join('');
        const result = evaluate(exprStr);
        const displayResult = formatNumber(result);

        next.expression = exprStr + ' =';
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

const engine = new StandardEngine();
registerMode('standard', engine);

export { engine as standardEngine };
