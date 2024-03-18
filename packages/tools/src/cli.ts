import esbuild from 'esbuild'
import fs from 'node:fs'
import path from 'node:path'
import { CopyAsset, copyPlugin } from './copy-plugin.js'
import { HandleBuild } from './process.js'
import { BTRPlugin } from './plugin-interface.js'
import { pathToFileURL } from 'node:url'

interface Options {
  port: number
  out: string
  www?: string
  client?: string
  plugin?: string
}

function buildBaseOptions(entryPoint: string, out: string): esbuild.BuildOptions {
  return {
    entryPoints: [entryPoint],
    sourcemap: true,
    format: 'esm',
    target: 'esnext',
    outdir: out,
    loader: {
      '.png': 'dataurl'
    }
  }
}

async function buildWebOptions(entryPoint: string, options: Options): Promise<esbuild.BuildOptions> {
  const config = buildBaseOptions(entryPoint, options.out)
  const entryPointDir = path.dirname(entryPoint)
  const assets: CopyAsset[] = [
    {
      from: [`${entryPointDir}/**/*.html`, `${entryPointDir}/**/*.css`],
      to: [options.out],
      flatten: true,
    },
  ]

  if (options.www) {
    assets.push({
      from: [`${options.www}/**/*`],
      to: [options.out],
      flatten: true,
    })
  }

  let customPlugin: BTRPlugin | undefined
  if (options.plugin) {
    const pluginPathAbsolute = path.resolve(process.env['INIT_CWD'] || process.cwd(), options.plugin);
    const pluginPathURL = pathToFileURL(pluginPathAbsolute).href;
    const plugin = await import(pluginPathURL);
    customPlugin = plugin.default()
  }

  return {
    ...config,
    bundle: true,
    outdir: options.out,
    plugins: [
      copyPlugin(assets, customPlugin),
    ],
  }
}

function buildNodeOptions(entryPoint: string, out: string): esbuild.BuildOptions {
  const config = buildBaseOptions(entryPoint, out)
  return {
    ...config,
    platform: 'node',
  }
}

export async function HandleServe(entryPoint: string, options: Options) {
  const config = await buildWebOptions(entryPoint, options)
  const context = await esbuild.context(config)

  await context.watch()

  const { port } = await context.serve({
    servedir: options.out,
    port: options.port,
    onRequest: (args) => {
      console.log(`[request] ${args.path}`)
    },
  })

  console.log(`[  ready] http://localhost:${port}/`)
}

export async function HandleWebBuild(entryPoint: string, options: Options) {
  const config = await buildWebOptions(entryPoint, options)
  config.minify = true
  config.metafile = true

  const results = await esbuild.build(config).catch(() => process.exit(1))
  for (const output in results.metafile?.outputs) {
    const file = results.metafile?.outputs[output]
    if (file.entryPoint) {
      const normalizedOutput = path.normalize(output.replace(config.outdir as string, ''))
      console.log(normalizedOutput, `${file.bytes / 1000}kb`)
      file.imports.forEach((imported) => {
        const normalizedImport = path.normalize(imported.path.replace(config.outdir as string, ''))
        if (imported.path.search('node_modules') === -1) {
          console.log(normalizedImport, `${getFileSizeInBytes(imported.path) / 1000}kb`)
        }
      })
    }
  }
}

export async function HandleNodeBuild(entryPoint: string, options: Options) {
  const config = buildNodeOptions(entryPoint, options.out)
  await esbuild.build(config).catch(() => process.exit(1))
  HandleBuild(options.client!, { port: 3001, useLinkCss: true })
}

function getFileSizeInBytes(path: string): number {
  const stats = fs.statSync(path)
  const fileSizeInBytes = stats.size
  return fileSizeInBytes
}
