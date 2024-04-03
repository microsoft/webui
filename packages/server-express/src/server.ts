import { handleBTR } from '@btjs/parser-js'
import type { BuildTimeRenderingProtocol } from '@btjs/protocol-js'
import express from 'express'
import { existsSync, readFileSync } from 'node:fs'

export interface ServeStaticOptions {
  maxAge?: number | string
}

export class BTRServer {
  #app: express.Express
  #handlerMap = new Map<string, BuildTimeRenderingProtocol>()

  constructor(public appPath: string, public port: number, public options?: ServeStaticOptions) {
    this.#app = express()
    this.#app.use(express.json())
  }

  public async start() {
    // Default state should done at the end so the default endpoints are served first
    // for example when hitting `/` it should let BTR take control first instead of index.html.
    this.#app.use(express.static(this.appPath, { maxAge: this.options?.maxAge || '1d' }))
    this.#app.listen(this.port, async () => {
      console.log(`BTR Server is listening on port http://localhost:${this.port}`)
    })
  }

  public get app() {
    return this.#app
  }

  public addHandler(method: 'get' | 'post', path: string, protocolFile: string, state: Object) {
    if (!existsSync(protocolFile)) {
      throw new Error(`No BTR protocol file found: "${protocolFile}"`)
    }

    const handlerKey = `${method}:${path}`
    if (this.#handlerMap.has(handlerKey)) {
      throw new Error(`Handler already exists for ${handlerKey}`)
    }

    try {
      const streamingProtocol = JSON.parse(readFileSync(protocolFile, 'utf8')) as BuildTimeRenderingProtocol
      this.#handlerMap.set(handlerKey, streamingProtocol)
      this.#app[method](path, (_req, res) => {
        console.time(handlerKey)
        handleBTR(streamingProtocol, state, res)
        console.timeEnd(handlerKey)
      })
    } catch (e) {
      throw new Error('Invalid BTR protocol file')
    }
  }
}
