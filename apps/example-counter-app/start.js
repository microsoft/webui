import esbuild from 'esbuild'
import copy from 'esbuild-plugin-copy'

const context = await esbuild.context({
  entryPoints: {
    'index': './src/index.js',
  },
  bundle: true,
  sourcemap: true,
  format: 'esm',
  target: 'esnext',
  outdir: './dist',
  plugins: [
    copy({
      resolveFrom: 'cwd',
      assets: [
        {
          from: ['./www/*'],
          to: ['./dist'],
        },
        {
          from: ['./src/**/*.html', './src/**/*.css'],
          to: ['./dist'],
        },
      ],
    }),
  ],
})

await context.watch()

const { host, port } = await context.serve({
  servedir: './dist',
  port: 3001,
  onRequest: (args) => {
    console.log(`[request] ${args.path}`)
  },
})

console.log(`[  ready] http://localhost:${port}/`)
