import { BTRServer, StateFile } from '@btjs/server-express'
import { Command } from 'commander'
import { join } from 'node:path'

export function HandleServe(appPath: string, port: number) {
  const server = new BTRServer(appPath, port)
  const state = new StateFile(appPath, {
    items: [],
    appTitle: 'Welcome Build Time Rendering (BTR)!',
  })

  // Create API endpoint to handle POST requests.
  server.app.post('/api/items', (req, res) => {
    res.status(201).json(req.body)
    state.data['items'] = req.body
    state.flush()
  })

  // Create BTR endpoint for /
  server.addHandler('get', '/', join(appPath, 'index.streams.json'), state.data)

  // Start the server.
  server.start()
}

const program = new Command()
program
  .name('BTR Todo App')
  .version('0.0.1')
  .option('-p, --port <port>', 'Port to serve the app', (value) => parseInt(value, 10), 3000)
  .arguments('<path>')
  .action((path, options) => HandleServe(path, options.port))
program.parse(process.argv)
