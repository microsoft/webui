import { Command } from 'commander'
import { HandleBuild } from './process.js'

const program = new Command()
program
  .name('FAST BTR Build')
  .description('Build Time Rendering ')
  .version('0.0.1')
  .option('-p, --port <port>', 'Port to serve the app', (value) => parseInt(value, 10), 3000)
  .option('--use-link-css', 'Use the css contents from external link tag', false)
  .arguments('<path>')
  .action((path, options) => HandleBuild(path, options))
program.parse(process.argv)
