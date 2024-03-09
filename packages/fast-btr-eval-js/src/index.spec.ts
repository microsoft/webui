import assert from 'assert'
import { parseExpression, safeEvaluateExpression } from './index'

describe('safeEvaluateExpression', function() {
  it('should parse expressions correctly', function() {
    const testCases = [
      { expression: 'a && b || c', expected: ['a', '&&', 'b', '||', 'c'] },
      { expression: 'd == e', expected: ['d', '==', 'e'] },
      { expression: 'f > g', expected: ['f', '>', 'g'] },
      { expression: 'k&&l', expected: ['k', '&&', 'l'] },
      { expression: 'm  ||  n', expected: ['m', '||', 'n'] },
      {
        expression: 'o > p && q <= r || s == t',
        expected: ['o', '>', 'p', '&&', 'q', '<=', 'r', '||', 's', '==', 't'],
      },
      { expression: 'foo.bar && bar', expected: ['foo.bar', '&&', 'bar'] },
    ]

    for (const testCase of testCases) {
      assert.deepEqual(parseExpression(testCase.expression), testCase.expected, `Expression: ${testCase.expression}`)
    }
  })

  it('should evaluate expressions correctly', function() {
    const state = {
      name: {
        first: 'John',
        last: 'Doe',
      },
      favorite: {
        categories: {
          movies: ['The Matrix', 'The Godfather'],
          music: ['Jazz', 'Blues'],
        },
      },
      age: 30,
      is_student: true,
    }

    const testCases = [
      { expression: 'name.first && age', expected: true },
      { expression: 'age == 30', expected: true },
      { expression: 'favorite.categories.music && favorite.categories.movies', expected: true },
      { expression: 'favorite.categories.music == favorite.categories.music', expected: true },
      { expression: 'age > 10', expected: true },
      { expression: 'name && name.first', expected: true },
      { expression: 'is_student', expected: true },
      { expression: 'is_student && name.first', expected: true },
      { expression: '!is_student || true', expected: true },
      { expression: '"a" == "a"', expected: true },
      { expression: 'favorite.categories.music.length > 0', expected: true },
      { expression: 'name.first &&', expected: false },
      { expression: 'name.middle', expected: false },
      { expression: 'name.first && (age == 31)', expected: false },
      { expression: 'name.first && false', expected: false },
      { expression: 'favorite.categories.music.length > 2', expected: false },
      { expression: 'favorite.categories.music == favorite.categories.movies', expected: false },
      { expression: '"a" == "b"', expected: false },
      { expression: '!is_student', expected: false },
      { expression: '!is_student && name.first', expected: false },
    ]

    for (const testCase of testCases) {
      assert.strictEqual(
        safeEvaluateExpression(parseExpression(testCase.expression), state),
        testCase.expected,
        `Expression: ${testCase.expression}`,
      )
    }
  })

  it('should not mutate the expression array', function() {
    const expressionArray = ['a']
    safeEvaluateExpression(expressionArray, { a: true })
    assert.deepEqual(expressionArray, ['a'])
  })
})
