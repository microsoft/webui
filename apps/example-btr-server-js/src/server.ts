import { handleBTR } from '@internal/fast-btr-parser-js'
import type { BuildTimeRenderingProtocol } from '@internal/fast-btr-protocol-js'
import express from 'express'
import { existsSync, readFileSync, writeFile } from 'node:fs'
import { join, resolve } from 'node:path'

export function HandleServe(appPath: string, port: number) {
  appPath = resolve(process.env['INIT_CWD'] || process.cwd(), appPath)

  const dbPath = join(appPath, 'db.json')
  const indexStreamsPath = join(appPath, 'index.streams.json')

  let db: Record<string, {}> = {
    items: [],
    appTitle: 'Welcome Build Time Rendering (BTR)!',
  }
  if (existsSync(dbPath)) {
    db = JSON.parse(readFileSync(dbPath, 'utf8'))
  }

  const app = express()
  app.use(express.json())

  let streamingProtocol: BuildTimeRenderingProtocol | undefined
  if (existsSync(indexStreamsPath)) {
    streamingProtocol = JSON.parse(readFileSync(indexStreamsPath, 'utf8')) as BuildTimeRenderingProtocol
  }

  app.post('/api/items', (req, res) => {
    res.status(201).json(req.body)
    db['items'] = req.body
    writeFile(dbPath, JSON.stringify(db), () => {})
  })

  app.get('/', (_req, res) => {
    if (!streamingProtocol) {
      res.send({ error: 'No BTR protocol file found' })
      return
    }

    handleBTR(streamingProtocol, db, res)
  })

  app.use(express.static(appPath))

  app.listen(port, async () => {
    console.log(`BTR Server is listening on port http://localhost:${port}`)
  })
}
