import { findValueByDottedPath, parseExpression, safeEvaluateExpression } from '@btjs/eval-js'

import type { BuildTimeRenderingProtocol, BuildTimeRenderingStream } from '@btjs/protocol-js'

export interface ServerHandler {
  write: (value: string) => void
  end: () => void
}

function escapeHtml(text: string): string {
  const map: { [key: string]: string } = {
    '&': '&amp;',
    '<': '&lt;',
    '>': '&gt;',
    '"': '&quot;',
    "'": '&#039;',
  }
  return text.replace(/[&<>"']/g, (m) => map[m])
}

export function handleBTR(protocol: BuildTimeRenderingProtocol, state: Object, serverHandler: ServerHandler) {
  const stack: { streamArray: BuildTimeRenderingStream[], index: number, state: Object }[] = []
  serverHandler.write('<!DOCTYPE html><html>')
  // Initialize the stack with the first call
  stack.push({ streamArray: protocol.streams, index: 0, state })

  while (stack.length > 0) {
    const { streamArray, index, state } = stack.pop()!

    if (index >= streamArray.length) continue

    const stream = streamArray[index]

    switch (stream.type) {
      case 'raw': {
        serverHandler.write(stream.value)
        break
      }
      case 'attribute': {
        const value = findValueByDottedPath(stream.value, state)
        if (value !== undefined) {
          serverHandler.write(`${stream.key}="${value}"`)
        }
        break
      }
      case 'signal': {
        let value = findValueByDottedPath(stream.value, state)
        if (!stream.raw) {
          value = escapeHtml(value)
        }
        if (value !== undefined) {
          serverHandler.write('' + value)
        } else if (stream.defaultValue !== undefined) {
          serverHandler.write(stream.defaultValue)
        }
        break
      }
      case 'repeat': {
        const value = findValueByDottedPath(stream.value, state)
        if (value === undefined) {
          console.error(`Repeat value not found: ${stream.value}`)
          break
        }
        const template = protocol.templates[stream.template]

        // Push the remaining elements of the current array back onto the stack
        stack.push({ streamArray, index: index + 1, state })

        // Process the repeated items by pushing each one onto the stack
        for (let i = value.length - 1; i >= 0; i--) {
          stack.push({ streamArray: template, index: 0, state: value[i] })
        }
        continue
      }
      case 'when': {
        const parts = parseExpression(stream.value)
        const value = safeEvaluateExpression(parts, state)
        if (!value) {
          serverHandler.write('style="display: none"')
        }
        break
      }
      case 'component': {
        const template = protocol.templates[stream.value]
        stack.push({ streamArray, index: index + 1, state })
        stack.push({ streamArray: template, index: 0, state })
        if (stream.css) {
          stack.push({
            streamArray: [{
              type: 'raw',
              value: `<link rel="stylesheet" href="./${stream.css}">`,
            }],
            index: 0,
            state,
          })
        }
        continue
      }
    }

    // Push the next item of the current array onto the stack
    stack.push({ streamArray, index: index + 1, state })
  }

  serverHandler.write('</html>')
  serverHandler.end()
}
