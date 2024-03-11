import esbuild from 'esbuild'
import copy from 'esbuild-plugin-copy'

esbuild.build({
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
}).catch(() => process.exit(1))
