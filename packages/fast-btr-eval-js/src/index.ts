const OPERATORS_ARRAY = ['&&', '||', '==', '>=', '<=', '>', '<', '!', '(', ')']
const OPERATOR_REGEX = new RegExp(
  `(${OPERATORS_ARRAY.map(op => op.replace(/\|/g, '\\|').replace(/\(/g, '\\(').replace(/\)/g, '\\)')).join('|')})`,
)

export const OPERATORS = new Set(OPERATORS_ARRAY)

/**
 * Finds a value by a dotted path.
 *
 * @param part The dotted path to the value
 * @param component The component to search for the value.
 * @returns The value found at the dotted path.
 */
export function findValueByDottedPath(part: string | number, state: Object) {
  let value: any | undefined

  if (isNaN(Number(part))) {
    const properties = (part as string).split('.')
    value = state
    for (const property of properties) {
      value = value[property]
      if (value === undefined) {
        break
      }
    }
  } else {
    value = Number(part)
  }

  return value
}

export function parseExpression(expression: string): string[] {
  return expression.split(OPERATOR_REGEX).filter(part => part && part.trim().length).map(s => s.trim())
}

export function safeEvaluateExpression(expression: string[], state: any): boolean {
  const parts = expression.slice().reverse()
  return evaluate(parts, state)
}

function evaluate(tokens: string[], state: any): any {
  let currentValue = null
  let currentOperator = null

  while (tokens.length > 0) {
    let token = tokens.pop()

    switch (token) {
      case '(': {
        let value = evaluate(tokens, state)
        currentValue = applyOperator(currentValue, currentOperator, value)
        currentOperator = null
        break
      }
      case ')':
        break
      case '==':
      case '!=':
      case '<':
      case '<=':
      case '>':
      case '>=':
      case '&&':
      case '||':
      case '!': {
        if (currentOperator !== null) {
          let value = evaluate(tokens, state)
          currentValue = applyOperator(currentValue, currentOperator, value)
        }
        currentOperator = token
        break
      }
      default: {
        let value = parseValue(token!, state)
        currentValue = applyOperator(currentValue, currentOperator, value)
        currentOperator = null
        break
      }
    }
  }

  if (currentOperator !== null) {
    let value = evaluate(tokens, state)
    currentValue = applyOperator(currentValue, currentOperator, value)
  }

  return currentValue
}

function applyOperator(value1: any, operator: string | null, value2: any): any {
  switch (operator) {
    case '==':
      return Boolean(value1 == value2)
    case '!=':
      return Boolean(value1 != value2)
    case '>':
      return Boolean(value1 > value2)
    case '<':
      return Boolean(value1 < value2)
    case '>=':
      return Boolean(value1 >= value2)
    case '<=':
      return Boolean(value1 <= value2)
    case '&&':
      return Boolean(value1 && value2)
    case '||':
      return Boolean(value1 || value2)
    case '!':
      return Boolean(!value2)
    default:
      return value2
  }
}

function parseValue(token: string, state: any): any {
  if (!isNaN(Number(token))) {
    return Number(token)
  }
  if (token.startsWith('"') || token.startsWith("'")) {
    return token.slice(1, -1)
  }
  switch (token) {
    case 'true':
      return true
    case 'false':
      return false
    default:
      return findValueByDottedPath(token, state) || false
  }
}
