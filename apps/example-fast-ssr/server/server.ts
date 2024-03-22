import '@microsoft/fast-ssr/install-dom-shim.js'
import { BTRServer, StateFile } from '@btjs/server-express'
import fastSSR from '@microsoft/fast-ssr'
import { createWindow, installWindowOnGlobal } from '@microsoft/fast-ssr/dom-shim'
import { Command } from 'commander'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { FASTElement, css, customElement, html } from '@microsoft/fast-element'

const template = html<Counter>`
  <span>${x => x.counter}</span>
  <button @click=${x => x.onDecrementClick()}>-</button>
  <button @click=${x => x.onIncrementClick()}>+</button>
  <button @click=${x => x.onResetClick()}>reset</button>
`

const styles = css`
:host {
  background-color: #ddd;
  margin: 10px;
  padding: 10px;
  display: inline-flex;
  align-items: center;
  gap: 10px;
  border-radius: 10px;
  user-select: none;
}

button {
  color: black;
  padding: 10px;
  border: none;
  cursor: pointer;
  border-radius: 10px;
  &:hover {
    outline: 1px solid #bbb;
    background-color: #ccc;
  }
}

span {
  font-size: 20px;
  font-family: 'Courier New', Courier, monospace;
  min-width: 50px;
  text-align: center;
}
`

@customElement({ name: 'counter-element', template, styles })
export class Counter extends FASTElement {
}

installWindowOnGlobal(createWindow({ isSSR: true }))
const { templateRenderer } = fastSSR()

export function HandleServe(appPath: string, port: number) {
  const server = new BTRServer(appPath, port)
  const state = new StateFile(appPath, {
    counter: 0,
  })
  const fileContent = readFileSync(join(appPath, 'index.html'), 'utf-8')

  server.app.post('/api/update', (req, res) => {
    res.status(201).json(req.body)
    state.data['counter'] = req.body.counter || 0
    state.flush()
  })

  server.app.get('/', (_req, res) => {
    console.time('render')
    res.type('html')
    const result = templateRenderer.render(fileContent, templateRenderer.createRenderInfo(), state.data)
    for (const value of result) {
      res.write(value.toString())
    }
    res.end()
    console.timeEnd('render')
  })

  server.start()
}

const program = new Command()
program
  .name('FAST-SSR Counter App')
  .version('0.0.1')
  .option('-p, --port <port>', 'Port to serve the app', (value) => parseInt(value, 10), 3000)
  .arguments('<path>')
  .action((path, options) => HandleServe(path, options.port))
program.parse(process.argv)
