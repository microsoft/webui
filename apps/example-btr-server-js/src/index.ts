import { Command } from 'commander'
import { HandleServe } from './server.js'

const program = new Command()
program
  .name('FAST BTR Build')
  .description('Server for Build Time Rendering')
  .version('0.0.1')
  .option('-p, --port <port>', 'Port to serve the app', (value) => parseInt(value, 10), 3000)
  .arguments('<path>')
  .action((path, options) => HandleServe(path, options.port))
program.parse(process.argv)
