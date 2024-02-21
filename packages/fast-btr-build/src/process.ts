import type {
  BuildTimeRenderingProtocol,
  BuildTimeRenderingStream,
  BuildTimeRenderingStreamTemplateRecords,
} from '@internal/fast-btr-protocol'
import express from 'express'
import { existsSync, readFile, writeFile } from 'node:fs'
import { join, resolve } from 'node:path'
import { chromium } from 'playwright'
import { extractHTML } from './extract.js'

export function HandleBuild(appPath: string, port: number) {
  appPath = resolve(process.env['INIT_CWD'] || process.cwd(), appPath)

  if (!existsSync(appPath)) {
    console.error('App not found:', appPath)
    process.exit(1)
  }

  const app = express()
  app.use(express.static(appPath))

  let streamResponses: BuildTimeRenderingStream[] = []
  let streamTemplates: BuildTimeRenderingStreamTemplateRecords = {}

  const server = app.listen(port, async () => {
    console.log('preparing browser')

    const browser = await chromium.launch({
      channel: 'msedge',
    })
    const page = await browser.newPage()

    page.on('console', msg => {
      if (msg.type() === 'error') {
        console.log('ERROR:', msg.text())
      } else {
        console.log('LOG:', msg.text())
      }
    })

    console.log('visiting app')
    await page.goto(`http://localhost:${port}`)

    await page.exposeFunction('readFile', (path: string) => {
      return new Promise((resolve, reject) => {
        readFile(join(appPath, path), 'utf8', (err, data) => {
          if (err) {
            reject('Error reading file:' + err)
          } else {
            const content = data.trim()
            if (!content || content.length === 0) {
              resolve([])
            } else {
              resolve(content.split('\n'))
            }
          }
        })
      })
    })

    await page.exposeFunction('writeStreamResponse', (type: string, value: string, extra: string) => {
      if (type === 'repeat') {
        streamResponses.push({ type, value, template: extra })
      } else if (type === 'signal') {
        streamResponses.push({ type, value, defaultValue: extra })
      } else if (type === 'raw') {
        streamResponses.push({ type, value })
      } else if (type === 'when') {
        streamResponses.push({ type, value })
      } else {
        console.error('Unknown stream type:', type)
      }
    })

    await page.exposeFunction('writeTemplates', (templates: string) => {
      streamTemplates = JSON.parse(templates) as BuildTimeRenderingStreamTemplateRecords
    })

    console.log('dumping stream')
    const htmlLines = await page.evaluate(extractHTML, 'html')

    writeFile(join(appPath, 'index.debug.html'), htmlLines.join('\n'), (err) => {
      if (err) {
        console.error('Error writing debug file:', err)
      }
    })

    const streamProtocol: BuildTimeRenderingProtocol = {
      streams: streamResponses,
      templates: streamTemplates,
    }

    writeFile(join(appPath, 'index.streams.json'), JSON.stringify(streamProtocol, null, 2), (err) => {
      if (err) {
        console.error('Error writing streams file:', err)
      }
    })

    console.log('stream partials:', streamResponses.length)
    console.log('closing')
    await browser.close()
    server.close()
  })
}
