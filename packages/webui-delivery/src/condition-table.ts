// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

type Resolve = (path: string, scope?: unknown) => unknown;
export type Condition = (resolve: Resolve, scope: unknown, truthy: (value: unknown) => boolean) => boolean;
type Operand = (resolve: Resolve, scope: unknown) => unknown;
const CONDITION_SOURCE = Symbol.for('webui.condition-source');

// Keep the compiler's JavaScript comparison semantics, including coercion.
const comparisons: ReadonlyArray<readonly [string, (a: unknown, b: unknown) => boolean]> = [
  ['>=', (a, b) => (a as number) >= (b as number)],
  ['<=', (a, b) => (a as number) <= (b as number)],
  ['==', (a, b) => a == b],
  ['!=', (a, b) => a != b],
  ['>', (a, b) => (a as number) > (b as number)],
  ['<', (a, b) => (a as number) < (b as number)],
];

type Frame = null | number | { left: Condition; and: boolean };
// Supported compounds fit well below this bound once negation runs are folded.
const MAX_NESTING = 32;

/** Decode the condition table emitted by the compiler, never evaluating response text as JavaScript. */
export function readConditionTable(source: unknown, tag: string): Condition[] {
  if (typeof source !== 'string') throw invalidTable(tag);
  return new ConditionReader(source, tag).read();
}

class ConditionReader {
  private offset = 0;

  constructor(private readonly source: string, private readonly tag: string) {}

  read(): Condition[] {
    const table: Condition[] = [];
    this.expect('[');
    if (!this.take(']')) {
      do {
        const start = this.offset;
        this.expect('function(v,s');
        this.take(',t');
        this.expect('){return ');
        const condition = this.condition();
        this.expect('}');
        // The optional template cache compares these with SSR condition functions.
        Object.defineProperty(condition, CONDITION_SOURCE, { value: this.source.slice(start, this.offset) });
        table.push(condition);
      } while (this.take(','));
      this.expect(']');
    }
    if (this.offset !== this.source.length) throw invalidTable(this.tag);
    return table;
  }

  private condition(): Condition {
    const frames: Frame[] = [];
    for (;;) {
      for (;;) {
        // Consecutive negations need one frame and at most one closure.
        let negations = 0;
        while (this.take('!(')) negations++;
        if (negations === 0 && !this.take('(')) break;
        if (frames.length === MAX_NESTING) throw invalidTable(this.tag);
        frames.push(negations || null);
      }
      let value = this.term();
      while (frames.length) {
        const frame = frames.pop()!;
        if (frame === null) {
          const and = this.take('&&');
          if (!and) this.expect('||');
          frames.push({ left: value, and });
          break;
        }
        if (typeof frame === 'number') {
          for (let i = 0; i < frame; i++) this.expect(')');
          if (frame % 2 !== 0) {
            const inner = value;
            value = (v, s, t) => !inner(v, s, t);
          }
          continue;
        }
        this.expect(')');
        const right = value;
        value = frame.and
          ? (v, s, t) => frame.left(v, s, t) && right(v, s, t)
          : (v, s, t) => frame.left(v, s, t) || right(v, s, t);
      }
      if (frames.length === 0) return value;
    }
  }

  private term(): Condition {
    if (this.take('true') || this.take('!false')) return () => true;
    if (this.take('false')) return () => false;
    if (this.take('t(')) {
      const read = this.lookup();
      this.expect(')');
      return (v, s, t) => t(read(v, s));
    }
    const left = this.lookup();
    for (const [operator, compare] of comparisons) {
      if (this.take(operator)) {
        const right = this.operand();
        return (v, s) => compare(left(v, s), right(v, s));
      }
    }
    throw invalidTable(this.tag);
  }

  private operand(): Operand {
    if (this.source.startsWith('v(', this.offset)) return this.lookup();
    if (this.take('true')) return () => true;
    if (this.take('false')) return () => false;
    const value = this.source[this.offset] === '"' ? this.string() : this.number();
    return () => value;
  }

  private lookup(): Operand {
    this.expect('v(');
    const path = this.string();
    this.expect(',s)');
    return (v, s) => v(path, s);
  }

  private string(): string {
    const start = this.offset;
    this.expect('"');
    while (this.offset < this.source.length) {
      const ch = this.source[this.offset++];
      if (ch === '\\') this.offset++;
      else if (ch === '"') {
        try {
          return JSON.parse(this.source.slice(start, this.offset)) as string;
        } catch (cause) {
          throw invalidTable(this.tag, cause);
        }
      }
    }
    throw invalidTable(this.tag);
  }

  private number(): number {
    const start = this.offset;
    const negative = this.take('-');
    const digits = this.offset;
    let octal = true;
    while (this.digit()) {
      if (this.source[this.offset] > '7') octal = false;
      this.offset++;
    }
    const integerEnd = this.offset;
    const decimal = this.take('.');
    if (decimal) while (this.digit()) this.offset++;
    if (integerEnd === digits && (!decimal || this.offset === integerEnd + 1)) {
      throw invalidTable(this.tag);
    }
    // The existing compiler emits leading-zero literals into a classic script.
    if (octal && integerEnd - digits > 1 && this.source[digits] === '0') {
      if (decimal) throw invalidTable(this.tag);
      return (negative ? -1 : 1) * Number.parseInt(this.source.slice(digits, integerEnd), 8);
    }
    return Number(this.source.slice(start, this.offset));
  }

  private digit(): boolean {
    const code = this.source.charCodeAt(this.offset);
    return code >= 48 && code <= 57;
  }

  private take(token: string): boolean {
    if (!this.source.startsWith(token, this.offset)) return false;
    this.offset += token.length;
    return true;
  }

  private expect(token: string): void {
    if (!this.take(token)) throw invalidTable(this.tag);
  }
}

function invalidTable(tag: string, cause?: unknown): Error {
  return new Error(`[WebUI] Invalid compiler condition table for <${tag}>. Rebuild matching artifacts.`, { cause });
}
