import { findValueByDottedPath } from '@btjs/eval-js'


export async function processTemplate(source: string, state: Object) {
  const kLeader = '{{'
  const kTrailer = '}}'

  let formatted = ''
  let currentPos = 0

  while (true) {
    const nextPos = source.indexOf(kLeader, currentPos)

    if (nextPos === -1) {
      formatted += source.slice(currentPos)
      break
    }

    formatted += source.slice(currentPos, nextPos)
    currentPos = nextPos + kLeader.length

    const keyEnd = source.indexOf(kTrailer, currentPos)

    if (keyEnd === -1) {
      throw new Error('Template expression not closed')
    }

    const key = source.slice(currentPos, keyEnd)
    const value = findValueByDottedPath(key, state)
    if (value === undefined) {
      throw new Error(`Replacement key "${key}" not found`)
    }

    formatted += value
    currentPos = keyEnd + kTrailer.length
  }

  return formatted
}
