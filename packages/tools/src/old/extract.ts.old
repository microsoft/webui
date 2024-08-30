import type { BuildTimeRenderingStreamTemplateRecords } from '@btjs/protocol-js'

declare global {
  interface Window {
    readFile: (path: string) => Promise<string[]>
    writeStreamResponse: (type: string, value: string, extra?: string, detail?: string) => void
    writeTemplates: (value: string) => void
    writePreload: (type: string, value: string[], lines: string[]) => Promise<string[]>
  }

  interface HTMLElement {
    module?: string
  }
}

interface ExtractOptions {
  useLinkCss?: boolean
}

export async function extractHTML(options: ExtractOptions = {}): Promise<string[]> {
  const lines: string[] = []
  const hydratedModules: string[] = []
  let streamResponse: string[] = []
  const prefix = 'f-'
  const cssCache = new Map()
  const templateCache: BuildTimeRenderingStreamTemplateRecords = {}
  const preloadCss = new Set<string>()

  /**
   * Reads the contents of a CSS file from the file system. Caches the result.
   * @param {string} path - The path to the CSS file.
   */
  async function readCSS(path: string) {
    if (cssCache.has(path)) {
      return cssCache.get(path)
    }

    try {
      const contents = await window.readFile(path)
      cssCache.set(path, contents)
      return contents
    } catch (e) {
      console.error('Error reading CSS file', path, e)
      return []
    }
  }

  function addLine(line: string | undefined = undefined, indent = '') {
    if (!line) return
    lines.push(indent + line)
    streamResponse.push(indent + line)
  }

  function flushPlainResponse() {
    const streamContent = streamResponse.join('\n')
    streamResponse = []
    window.writeStreamResponse('raw', streamContent)
  }

  function flushStreamResponse(type: string, value: string, extra?: string, detail?: string) {
    flushPlainResponse()
    window.writeStreamResponse(type, value, extra?.trim(), detail?.trim())
  }

  function processWhenHint(node: HTMLElement) {
    // If the node has a `when` hint, clear the style attribute since it will be set by the protocol.
    node.style.display = ''
    if (node.style.cssText === '') {
      node.removeAttribute('style')
    }
  }

  async function processRepeatHint(node: HTMLElement) {
    // Special treatment for repeat hints, we need to extract the template and the style.
    // This is needed since css modules are not declarative. https://github.com/WICG/webcomponents/issues/939
    const tag = node.getAttribute('w-component')
    if (tag && !templateCache[tag]) {
      const repeatNode = document.createElement(tag)

      // Repeat templates should be kept in the body so that the shadowroot can reference
      // it when that tag is created. The reason, @customElements tag is evaluated right
      // at beginning before the custom element is registered, so it is expecting a template
      // to be available. Instead of fetching it from the server, we can keep it in the body.
      document.getElementById(tag)?.classList.remove('internal-html-module')

      // Add it to the body so the shadowroot is created.
      document.body.appendChild(repeatNode)

      const lines = []
      const templateNode = repeatNode.shadowRoot ? repeatNode.shadowRoot : repeatNode
      const callback: VisitCall = {
        addLine: (line: string | undefined, _indent?: string) => {
          if (!line || !line.trim()) return
          lines.push(line)
        },
        flushPlainResponse: () => {},
        flushStreamResponse: () => {},
      }

      let style = undefined
      if (repeatNode.module) {
        if (options.useLinkCss) {
          // callback.addLine(
          //   `<link rel="stylesheet" href="${repeatNode.module}">`,
          //   '  ',
          // )
          preloadCss.add(repeatNode.module)
        } else {
          const cssContents = await readCSS(`./${repeatNode.module}.css`)
          style = cssContents.join('\n')
          removeLinkTagIfExists(repeatNode)
        }
      } else {
        console.error('No CSS Module found for', tag)
      }

      for (const child of Array.from(templateNode.childNodes)) {
        if (child.nodeType === Node.ELEMENT_NODE) {
          await visitNode(child as HTMLElement, callback, '  ')
        } else {
          lines.push(child.textContent?.trim())
        }
      }

      templateCache[tag] = {
        template: lines.join('\n'),
        style,
      }

      return tag
    }
    return undefined
  }

  function removeLinkTagIfExists(node: HTMLElement) {
    const selectorNode = node.shadowRoot ? node.shadowRoot : node
    const linkCssExists = selectorNode.querySelector(
      `link[rel="stylesheet"][href="./${node.module}.css"]`,
    )
    if (linkCssExists) {
      linkCssExists.remove()
    }
  }

  interface VisitCall {
    addLine: (line: string | undefined, indent?: string) => void
    flushPlainResponse: () => void
    flushStreamResponse: (type: string, value: string, extra?: string, detail?: string) => void
  }

  /**
   * Visits a node and its children, printing the HTML representation of the node and its children.
   *
   * @param {HTMLElement} node - The node to visit
   * @param {string} indent - The current indentation level
   * @returns
   */
  async function visitNode(node: HTMLElement, callback: VisitCall, indent = '') {
    const nodeName = node.nodeName.toLowerCase()
    if (node.classList.contains('internal-html-module')) {
      // Skip repeat elements for now, this can be optimized later with embedded templates.
      // Reason of skipping it is that once the server has 0 items in this repeat, it doesn't know
      // what the HTML is because it isn't cached yet. Hydrated Modules assume the client has it already
      // in the DOM.
      hydratedModules.push(node.id)
      return lines
    }

    // Requirement for BTR (v1) is that script elements defined at the end.
    // Right before that, add all the hydrated modules to window scope so that
    // the client can know which elements are hydrated so it doesn't download the
    // html.
    if (nodeName === 'script' && hydratedModules.length > 0) {
      const hydratedString = JSON.stringify(hydratedModules)
      callback.addLine(`<script>window.btr = ${hydratedString};</script>`)
    }

    let tag = '<' + nodeName

    // Print attributes
    const attributeFastMap = new Map()
    const attributeBindingArray = []
    let repeatTemplate = undefined

    for (let i = 0; i < node.attributes.length; i++) {
      const name = node.attributes[i].name
      const value = node.attributes[i].value
      const hasPrefix = name.startsWith(prefix)

      // When the unprefixed attribute exists, skip the attribute without the prefix
      // since protocol will add it.
      if (hasPrefix || !node.attributes.getNamedItem(`f-${name}`)) {
        tag += ' ' + name + '="' + value + '"'
      }

      if (hasPrefix) {
        const isRepeatAttribute = name === (prefix + 'repeat')
        const isWhenAttribute = name === (prefix + 'when')
        const isSignalAttribute = name === (prefix + 'signal')
        const isRefAttribute = name === (prefix + 'ref')
        const isEventAttribute = name.startsWith(prefix + 'on')

        if (!isRepeatAttribute && !isWhenAttribute && !isSignalAttribute && !isRefAttribute && !isEventAttribute) {
          const attributeName = name.substring(prefix.length)
          const attributeValue = node.attributes.getNamedItem(attributeName)?.value
          attributeBindingArray.push({ name: attributeName, value, defaultValue: attributeValue })
          continue
        }

        if (isRepeatAttribute) {
          repeatTemplate = await processRepeatHint(node)
        }

        if (isWhenAttribute) {
          processWhenHint(node)
        }

        attributeFastMap.set(name, value)
      }
    }

    // For `when` hints, stream the protocol so that caller can set the style attribute.
    // For attribute bindings, stream the protocol so that caller can set the attribute.
    const hasWhenHint = attributeFastMap.has(prefix + 'when')
    const hasAttributeBindingsHint = attributeBindingArray.length > 0
    if (hasWhenHint || hasAttributeBindingsHint) {
      callback.addLine(tag, indent)
      if (hasAttributeBindingsHint) {
        attributeBindingArray.forEach(({ name, value, defaultValue }) => {
          callback.flushStreamResponse('attribute', value, name, defaultValue)
        })
      }
      if (hasWhenHint) {
        callback.flushStreamResponse('when', attributeFastMap.get(prefix + 'when'))
      }
      callback.addLine('>', indent)
    } else {
      callback.addLine(tag + '>', indent)
    }

    let tagContent = undefined

    // Print text content if it's a leaf node
    if (node.children.length === 0 && node.textContent?.trim() !== '') {
      tagContent = indent + ' ' + node.textContent!.trim()
    }

    // Stream Response construction.
    if (attributeFastMap.has(prefix + 'repeat')) {
      callback.flushStreamResponse('repeat', attributeFastMap.get(prefix + 'repeat'), repeatTemplate)
      callback.addLine(tagContent)
    } else if (attributeFastMap.has(prefix + 'signal')) {
      callback.flushStreamResponse('signal', attributeFastMap.get(prefix + 'signal'), tagContent)
    }

    // For non signal parent nodes, we need to recurse on children.
    // Signal nodes are not recursed since the core assumption is that they are leaf nodes.
    if (!attributeFastMap.has(prefix + 'signal')) {
      // Recurse on shadow root children
      if (node.shadowRoot) {
        callback.addLine('<template shadowrootmode="open">', indent + '  ')

        if (node.module) {
          if (options.useLinkCss) {
            preloadCss.add(node.module)
          } else {
            let cssContents = await readCSS(`./${node.module}.css`)
            if (cssContents && cssContents.length > 0) {
              // That is the best we can to eliminate flickering, by placing the contents of the CSS module in component.
              callback.addLine(
                `<style>\n${cssContents.map((line: string) => indent + '    ' + line.trim()).join('\n')}</style>`,
                indent + '    ',
              )
            }

            // <style> tags is used to eliminate flickering. We can remove the link tag if it exists.
            removeLinkTagIfExists(node)
          }

          // TODO: Doesn't work in Web Platform Yet.
          // addLine(indent + '  <script type="module">import sheet from "' + node.module + '" assert { type: "css" };console.log(this);</script>');
        }

        for (const child of Array.from(node.shadowRoot!.childNodes)) {
          if (child.nodeType === Node.ELEMENT_NODE) {
            await visitNode(child as HTMLElement, callback, indent + '    ')
          } else {
            callback.addLine(child.textContent?.trim(), indent + '    ')
          }
        }
        callback.addLine('</template>', indent + '  ')
      }

      // Recurse on normal children
      for (const child of Array.from(node.childNodes)) {
        if (child.nodeType === Node.ELEMENT_NODE) {
          await visitNode(child as HTMLElement, callback, indent + '  ')
        } else {
          callback.addLine(child.textContent?.trim(), indent + '  ')
        }
      }

      // If the node is a <template>, recurse on its content. Web Components that have an open shadowroot will not
      // have a <template> tag, so we don't need to worry about that case.
      if (nodeName === 'template') {
        // Templates are static html, store it in a div so that mutation could happen.
        // Discover what css module we are using, to determine whether links or styles
        // are needed.
        const nodeTemplate = document.createElement('div')
        nodeTemplate.appendChild((node as HTMLTemplateElement).content.cloneNode(true))
        nodeTemplate.module = node.getAttribute('module') === 'true' ? node.id : undefined
        if (nodeTemplate.module) {
          if (options.useLinkCss) {
            preloadCss.add(nodeTemplate.module)
          } else {
            let cssContents = await readCSS(`./${nodeTemplate.module}.css`)
            if (cssContents && cssContents.length > 0) {
              const templateStyle = document.createElement('style')
              templateStyle.textContent = cssContents.map((line: string) => indent + line.trim()).join('\n')
              nodeTemplate.prepend(templateStyle)
            }
            removeLinkTagIfExists(nodeTemplate)
          }
        }

        callback.addLine(nodeTemplate.innerHTML, indent + '  ')
        // TODO: Keep this here incase we need to recurse on the template content.
        // for (const child of Array.from((node as HTMLTemplateElement).content.childNodes)) {
        //   if (child.nodeType === Node.ELEMENT_NODE) {
        //     await visitNode(child as HTMLElement, callback, indent + '  ')
        //   } else {
        //     callback.addLine(child.textContent?.trim(), indent + '  ')
        //   }
        // }
      }
    }

    // Print closing tag if needed.
    if (nodeName !== 'link' && nodeName !== 'meta' && nodeName !== 'input' && nodeName !== 'img') {
      callback.addLine('</' + nodeName + '>', indent)
    }

    return lines
  }

  const root = document.querySelector<HTMLElement>('html')
  if (root) {
    await visitNode(root, { addLine, flushPlainResponse, flushStreamResponse })
  }

  flushPlainResponse()

  window.writeTemplates(JSON.stringify(templateCache, null, 2))

  // Web Platform doesn't support CSS Modules for css in JS.
  return window.writePreload('css', Array.from(preloadCss).map(c => `./${c}.css`), lines)
}
