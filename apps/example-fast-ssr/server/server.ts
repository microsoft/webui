import '@microsoft/fast-ssr/install-dom-shim.js'
import { html } from '@microsoft/fast-element'
import fastSSR from '@microsoft/fast-ssr'
import { createWindow, installWindowOnGlobal } from '@microsoft/fast-ssr/dom-shim'
import '../client/src/counter-element.js'

installWindowOnGlobal(createWindow({ isSSR: true }))
const { templateRenderer } = fastSSR()

console.time('render')
const result = templateRenderer.render(html`<counter-element></counter-element>`)
for (const value of result) {
  console.log(value)
}
console.timeEnd('render')
