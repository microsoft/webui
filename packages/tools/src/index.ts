#!/usr/bin/env node

import { Command, Option } from 'commander'
import { HandleNodeBuild, HandleServe, HandleWebBuild } from './cli.js'

const program = new Command()
program
  .name('FAST BTR Build')
  .description('Server for Build Time Rendering')
  .version('0.0.1')

program
  .command('dev-server <entryPoint>')
  .description('Start the development server')
  .option('-p, --port <port>', 'Port to serve the app', (value) => parseInt(value, 10), 3000)
  .option('--www <www>', 'Public directory to serve')
  .option('--plugin <pluginPath>', 'Custom plugin to run post build')
  .action((entryPoint, options) => HandleServe(entryPoint, options))

program
  .command('build-client <entryPoint>')
  .description('Build the app')
  .option('--www <www>', 'Public directory to serve')
  .option('--plugin <pluginPath>', 'Custom plugin to run post build')
  .action((entryPoint, options) => HandleWebBuild(entryPoint, options))

program
  .command('build-server <entryPoint>')
  .description('Build the btr server')
  .option('--client <client>', 'Client directory')
  .action((entryPoint, options) => HandleNodeBuild(entryPoint, options))

program.commands.forEach(cmd => {
  cmd.addOption(new Option('--out <out>', 'Output directory'))
})

program.parse(process.argv)
