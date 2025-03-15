import { BTRServer, StateFile } from '@btjs/server-express'
import { Command } from 'commander'
import { join } from 'node:path'

export function HandleServe(appPath: string, port: number) {
  const server = new BTRServer(appPath, port)
  const state = new StateFile(appPath, {
    counter: 0,
    url: 'https://picsum.photos/id/0/100/35',
  })

  // Create API endpoint to handle POST requests.
  server.app.post('/api/update', (req, res) => {
    res.status(201).json(req.body)
    state.data['counter'] = req.body.counter || 0
    state.flush()
  })

  // Create BTR endpoint for /
  server.addHandler('get', '/', join(appPath, 'index.streams.json'), state.data)

  // Start the server.
  server.start()
}

const program = new Command()
program
  .name('BTR Counter App')
  .version('0.0.1')
  .option('-p, --port <port>', 'Port to serve the app', (value) => parseInt(value, 10), 3000)
  .arguments('<path>')
  .action((path, options) => HandleServe(path, options.port))
program.parse(process.argv)
