import esbuild from 'esbuild'
import * as fs from 'fs'
import * as glob from 'glob'
import * as path from 'path'

import { BTRPlugin } from './plugin-interface.js'

export interface CopyAsset {
  from: string[]
  to: string[]
  flatten?: boolean
}

export function copyPlugin(copyPaths: CopyAsset[], plugin?: BTRPlugin): esbuild.Plugin {
  return {
    name: 'copy-plugin',
    setup(build) {
      build.onEnd(() => {
        const copyEndPlugin = plugin?.onCopyEnd();
        const copiedFiles = new Set<string>()
        copyPaths.forEach(({ from, to, flatten }) => {
          from.forEach((fromPath) => {
            to.forEach((toPath) => {
              glob.sync(fromPath).forEach(file => {
                const destinationPath = flatten
                  ? path.join(toPath, path.basename(file))
                  : path.join(toPath, path.relative(path.dirname(fromPath), file))
                if (copiedFiles.has(destinationPath)) {
                  throw new Error(
                    `File with the same name already exists in the destination directory: ${destinationPath}`,
                  )
                }
                copiedFiles.add(destinationPath)
                fs.mkdirSync(path.dirname(destinationPath), { recursive: true })
                fs.copyFileSync(file, destinationPath)
                copyEndPlugin?.addFile(destinationPath)
              })
            })
          })
        })
        copyEndPlugin?.process()
      })
    },
  }
}
