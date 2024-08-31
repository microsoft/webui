import { readFile, readdir, stat, writeFile } from 'node:fs/promises'
import { basename, extname, join, resolve } from 'node:path'
import { ComponentStore, WebComponentDefinition, createBuildTimeProtocol } from './replay_protocol.js'

interface BuildOptions {
  port: number
  useLinkCss: boolean
}

export async function HandleBuild(appPath: string, _: BuildOptions) {
  appPath = resolve(process.env['INIT_CWD'] || process.cwd(), appPath)
  try {
    await stat(appPath)
  } catch (error) {
    console.error('App not found:', appPath)
    process.exit(1)
  }

  const componentsStore = await discoverWebComponents(appPath)
  const indexHtmlContents = await readFile(join(appPath, 'index.html'), 'utf8')
  const streamProtocol = createBuildTimeProtocol(indexHtmlContents, componentsStore)

  try {
    await writeFile(join(appPath, 'index.streams.json'), JSON.stringify(streamProtocol, null, 2))
  } catch (error) {
    console.error('Error writing streams file:', error)
  }
}

async function discoverWebComponents(appPath: string): Promise<ComponentStore> {
  const result: Record<string, WebComponentDefinition> = {}
  const files = await readdir(appPath)

  await Promise.all(files.map(async file => {
    if (extname(file) === '.html' && file !== 'index.html') {
      const fileNameWithoutExt = basename(file, '.html')
      const filePath = join(appPath, file)

      const cssFile = `${fileNameWithoutExt}.css`
      const cssFilePath = join(appPath, cssFile)

      const [template, styles] = await Promise.all([
        readFile(filePath, 'utf-8'),
        stat(cssFilePath).then(() => cssFile).catch(() => undefined),
      ])

      result[fileNameWithoutExt] = { template, styles }
    }
  }))

  return result
}
