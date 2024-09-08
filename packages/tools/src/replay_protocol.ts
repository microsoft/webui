import { Lang, SgNode, html, kind } from '@ast-grep/napi'
import {
  BuildTimeRenderingProtocol,
  BuildTimeRenderingStream,
  BuildTimeRenderingStreamRaw,
  BuildTimeRenderingStreamSignal,
  BuildTimeRenderingStreamTemplateRecords,
} from '@btjs/protocol-js'

const Prefix = 'f-'
const AttributeEvent = 'on'
const AttributeRef = 'ref'
const PrefixLength = Prefix.length

const AttributeName = {
  Signal: `${Prefix}signal`,
  Repeat: `${Prefix}repeat`,
  When: `${Prefix}when`,
}

const ParseResponse = {
  Continue: true,
  Stop: false,
}

const defaultOptions: BuildTimeRenderingOptions = {
  templateRepeatCount: 0,
  preserveAttributes: true,
}

export class ParseError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'ParseError'
  }
}

export interface WebComponentDefinition {
  template: string
  styles?: string
}

export interface BuildTimeRenderingOptions {
  templateRepeatCount?: number
  preserveAttributes?: boolean
}

export type ComponentStore = Record<string, WebComponentDefinition>

function parse(
  node: SgNode,
  componentStore: ComponentStore,
  options: Partial<BuildTimeRenderingOptions>,
): BuildTimeRenderingProtocol {
  let raw: Array<string> = []
  const protocolMessages: Array<BuildTimeRenderingStream> = []
  let protocolTemplates: BuildTimeRenderingStreamTemplateRecords = {}
  let processProtocolMessageAfterTag: BuildTimeRenderingStream | undefined = undefined

  function flush() {
    if (raw.length) {
      protocolMessages.push({
        type: 'raw',
        value: raw.join(''),
      })
      raw = []
    }
  }

  /**
   * Writes a raw text to the protocol messages.
   */
  function writeRaw(text: string) {
    raw.push(text)
  }

  function writeProtocol(protocol: BuildTimeRenderingStream) {
    flush()
    protocolMessages.push(protocol)
    processProtocolMessageAfterTag = undefined
  }

  function queueProtocolAfterTag(protocol: BuildTimeRenderingStream) {
    processProtocolMessageAfterTag = protocol
  }

  function handleSignal(value: SgNode) {
    queueProtocolAfterTag({
      type: 'signal',
      value: value.text(),
    })
    return ParseResponse.Stop
  }

  function handleRepeat(value: SgNode) {
    options.templateRepeatCount = (options.templateRepeatCount || 0) + 1

    const element = findClosestAncestor(value, 'element')
    const templateId = `repeat-${options.templateRepeatCount}`
    if (!element) {
      console.error('Repeat directive must be inside an element')
      return ParseResponse.Stop
    }

    // Make sure there are children elements.
    const firstChild = element.child(1)
    if (!firstChild || firstChild.kind() === 'end_tag') {
      console.error('Repeat directive must have a child element')
      return ParseResponse.Stop
    }

    // Queue the protocol message to be written after the tag is closed.
    queueProtocolAfterTag({
      type: 'repeat',
      value: value.text(),
      template: templateId,
    })

    // Parse the children of the element and store the template.
    const parsedTemplate = parse(firstChild, componentStore, options)
    protocolTemplates[templateId] = parsedTemplate.streams
    if (parsedTemplate.templates) {
      protocolTemplates = { ...protocolTemplates, ...parsedTemplate.templates }
    }
    return ParseResponse.Stop
  }

  function handleWhen(value: SgNode) {
    writeProtocol({
      type: 'when',
      value: value.text(),
    })
    return ParseResponse.Continue
  }

  function handleAttribute(name: string, value: SgNode) {
    if (!name.startsWith(AttributeEvent) && name !== AttributeRef) {
      writeProtocol({
        type: 'attribute',
        key: name,
        value: value.text(),
      })
    }
    return ParseResponse.Continue
  }

  /**
   * Finds the closest ancestor of a node with a specific kind.
   */
  function findClosestAncestor(node: SgNode, kind: string) {
    let parent = node.parent()
    while (parent) {
      if (parent.kind() === kind) return parent
      parent = parent.parent()
    }
    return null
  }

  /**
   * Parses the attributes by delegating to the appropriate handler based on the attribute name.
   * When the parser should stop parsing the element children, it returns false, this is
   * useful to delegate to the parser to handle the protocol message after the tag is closed.
   */
  function parseAttributes(node: SgNode) {
    const name = node.find(kind(Lang.Html, 'attribute_name'))
    const value = node.find(kind(Lang.Html, 'attribute_value'))
    /**
     * @type {ParseResponse}
     */
    let shouldContinueParsingElementChildren = ParseResponse.Continue
    if (name && value) {
      const nameText = name.text()
      if (nameText.startsWith(Prefix)) {
        if (options.preserveAttributes) {
          writeRaw(` ${nameText}="${value.text()}" `)
        }

        // Handle special attributes.
        switch (nameText) {
          case AttributeName.Signal: {
            shouldContinueParsingElementChildren = handleSignal(value)
            break
          }
          case AttributeName.Repeat: {
            shouldContinueParsingElementChildren = handleRepeat(value)
            break
          }
          case AttributeName.When: {
            shouldContinueParsingElementChildren = handleWhen(value)
            break
          }
          default: {
            shouldContinueParsingElementChildren = handleAttribute(
              nameText.substring(PrefixLength),
              value,
            )
            break
          }
        }
      } else {
        // Write the attribute as is.
        writeRaw(` ${node.text()} `)
      }
    }
    return shouldContinueParsingElementChildren
  }

  function parseHandlebars(text: string) {
    const signals = []
    let i = 0
    let lastIndex = 0

    while (i < text.length) {
      if (text[i] === '{' && text[i + 1] === '{') {
        let allowRawHtml = false
        let start = i
        let end = i + 2

        if (text[i + 2] === '{') {
          allowRawHtml = true
          end++
        }

        while (
          end < text.length &&
          !(text[end] === '}' && text[end + 1] === '}' && (!allowRawHtml || (allowRawHtml && text[end + 2] === '}')))
        ) {
          end++
        }

        if (end < text.length) {
          // Write raw text before the handlebars
          if (start > lastIndex) {
            const rawText = text.slice(lastIndex, start)
            writeRaw(rawText)
          }

          const value = text.slice(start + (allowRawHtml ? 3 : 2), end).trim()
          const signal: BuildTimeRenderingStreamSignal = {
            type: 'signal',
            value: value,
          }

          // Intentionlly after initializing signal to remove undefined.
          if (allowRawHtml) {
            signal.raw = true
          }

          writeProtocol(signal)

          i = end + (allowRawHtml ? 3 : 2)
          lastIndex = i
        } else {
          i++
        }
      } else {
        i++
      }
    }

    // Write any remaining raw text
    if (lastIndex < text.length) {
      const rawText = text.slice(lastIndex)
      writeRaw(rawText)
    }
  }

  function parseText(node: SgNode) {
    parseHandlebars(node.text())
    return ParseResponse.Stop
  }

  function parseComponent(tagName: SgNode, node: SgNode, component: WebComponentDefinition) {
    const tagNameText = tagName.text()

    // Check if the component has already been parsed. This is to avoid parsing the same
    // component multiple times.
    if (!protocolTemplates[tagNameText]) {
      // Parse the component and store the protocol messages.
      const parsedTemplate = parse(html.parse(component.template).root(), componentStore, options)
      protocolTemplates[tagNameText] = parsedTemplate.streams

      // Merge the templates if any exists.
      if (parsedTemplate.templates) {
        for (const [key, value] of Object.entries(parsedTemplate.templates)) {
          if (!(key in protocolTemplates)) {
            protocolTemplates[key] = value
          }
        }
      }
    }

    // Handle the components attributes.
    const hasAttributes = tagName.next()?.kind() === 'attribute'
    if (hasAttributes) {
      writeRaw(`<${tagNameText}`)
      const attributes = tagName.parent()?.findAll(kind(Lang.Html, 'attribute'))!
      for (const attribute of attributes) {
        parseAttributes(attribute)
      }
      writeRaw('>')
    } else {
      writeRaw(`<${tagNameText}>`)
    }

    // Write the component template and close the tag.
    writeRaw(`<template shadowrootmode="open">`)
    writeProtocol({
      type: 'component',
      value: tagNameText,
      css: component.styles,
    })
    writeRaw(`</template>`)

    const children = node.children()
    for (const child of children) {
      parseNode(child)
    }

    return ParseResponse.Stop
  }

  function parseTag(node: SgNode) {
    const tagName = node.find(kind(Lang.Html, 'tag_name'))
    if (!tagName) {
      console.error('Tag name not found')
      return ParseResponse.Continue
    }
    const tagNameText = tagName.text()

    // Check if any component is found in the component store.
    const component = componentStore[tagNameText]
    if (component) {
      return parseComponent(tagName, node, component)
    }

    const hasAttributes = tagName.next()?.kind() === 'attribute'
    if (hasAttributes) {
      // Write the opening tag name without the closing HTML since we are expecting
      // attributes to write.
      writeRaw(`<${tagNameText}`)

      // Parse the attributes, needs some special handling to determine when the parser
      // should stop parsing the elements children, delegating to the parser.
      const attributes = tagName.parent()?.findAll(kind(Lang.Html, 'attribute'))!
      let shouldContinueParsingElementChildren = ParseResponse.Continue
      for (const attribute of attributes) {
        if (parseAttributes(attribute) === ParseResponse.Stop) {
          shouldContinueParsingElementChildren = ParseResponse.Stop
        }
      }

      // Close the opening HTML since attributes are done parsing.
      writeRaw('>')

      // When parser decided to stop parsing, see if there is a protocol message
      // to write, then close the tag.
      if (!shouldContinueParsingElementChildren) {
        if (processProtocolMessageAfterTag) {
          writeProtocol(processProtocolMessageAfterTag)
        }
        writeRaw(`</${tagNameText}>`)
      }
      return shouldContinueParsingElementChildren
    } else {
      // No attributes, write the opening tag.
      writeRaw(`<${tagNameText}>`)
      return ParseResponse.Continue
    }
  }

  function parseNode(node: SgNode) {
    switch (node.kind()) {
      case 'ERROR':
        throw new ParseError(`Invalid: ${node.text()}`)
      case 'style_element':
      case 'script_element':
      case 'element':
        if (parseTag(node)) break
        else return
      case 'end_tag':
        writeRaw(node.text())
        break
      case 'raw_text':
      case 'doctype':
      case 'text':
        if (parseText(node)) break
        else return
      default:
        break
    }
    // If needed from the parser, continue parsing the children of the node.
    const children = node.children()
    for (const child of children) {
      parseNode(child)
    }
  }

  parseNode(node)
  flush()
  return { streams: protocolMessages, templates: protocolTemplates }
}

export function createBuildTimeProtocol(
  htmlContent: string,
  componentStore: ComponentStore = {},
  options: Partial<BuildTimeRenderingOptions> = {},
): BuildTimeRenderingProtocol {
  const ast = html.parse(htmlContent)
  const root = ast.root()
  options = { ...defaultOptions, ...options }
  return parse(root, componentStore, options)
}
