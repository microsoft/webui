# Client builds and live reload

Use `webui dev` for warm JavaScript/TypeScript builds, native WebUI rendering,
and SSE live reload in one command:

```text
webui dev ./src --state ./data/state.json --port 4000
```

The defaults are `index.html`, client entry `index.ts`, the `webui` plugin,
and watching enabled. The HTML should reference the compiled module, for example
`<script type="module" src="/index.js"></script>`. Client output is kept in
an isolated run directory beneath `node_modules/.cache/webui-dev`, separate
from production output and other dev servers. It is created automatically and
removed on orderly shutdown. Explicit `--servedir` directories are never
removed by WebUI. Use `--servedir` to choose a different output/static
asset directory, `--client-entry` to select another JS/TS input relative to APP,
and `--no-watch` for a one-time build followed by serving.

Node.js and a project-installed esbuild are required. Esbuild is resolved from
the app directory through normal Node package resolution; the CLI does not
invoke a global esbuild command. A missing dependency produces an actionable error; WebUI does not
install packages or require the TypeScript compiler for ordinary bundling.
The built-in builder uses one esbuild context with ESM bundling, source maps,
an ES2022 target, the project's tsconfig, and `__WEBUI_DEV__=true`.

WebUI watches the app, local components, and the enclosing project directory
(the nearest ancestor containing `package.json`). Outputs, `node_modules`,
and other ignored subtrees do not cause build loops. For source dependencies
outside those roots, repeat `--watch-path ../shared`. Watch paths are relative
to APP; directories must exist at startup.

The existing `webui serve` command keeps its native-only defaults. To opt into
built-in client bundling with that command, supply
`--client-entry index.ts --servedir dist --watch`.

## Custom build integrations

`--client-builder` is an escape hatch for applications that need custom
esbuild options, plugins, or awaited state/theme/projection generation:

```text
webui dev src --port 4000 --servedir dist --client-builder ./dev-builder.mjs
```

Do not also supply `--client-entry`. The module path is relative to the CLI's
working directory. The same argument interface works on Windows, Linux, and
macOS. With `webui serve`, create the output directory first and explicitly
enable `--watch`; `webui dev` creates it and enables watching by default.

## Builder module

The ES module must default-export an async factory. Its context contains
absolute application and output directories:

```js
import * as esbuild from "esbuild";

export default async function createBuilder({ appDir, outDir }) {
  const context = await esbuild.context({
    absWorkingDir: appDir,
    entryPoints: ["index.ts"],
    outdir: outDir,
    bundle: true,
    format: "esm",
  });

  return {
    // Optional additional input files or directories, relative to appDir.
    watchPaths: ["../shared"],
    async rebuild() {
      await context.rebuild();
      // Await all additional output, state, theme, and projection generation.
    },
    async dispose() {
      await context.dispose();
    },
  };
}
```

Use your application's normal esbuild settings, including any required
framework aliases, decorator settings, and plugins. Install esbuild in the
project that owns the module; WebUI does not install a bundler.

The factory runs once. Both `rebuild` and `dispose` are required and awaited.
Throw if a build fails, including failures after emitting files. Do not catch
an error and return success or treat an older bundle as a successful build.
The factory is responsible for cleaning up partially created resources if
initialization throws before returning its hooks.

`--watch` already watches TypeScript inside the app and local component roots.
Use `watchPaths` for additional dependencies, not a second `context.watch()`.
Paths are fixed for the lifetime of the server. Changes to the builder module
require restarting the server; the module is not hot-reloaded.

All generated outputs and temporary files must stay under `--servedir`.
The output directory must not contain the app, builder module, component roots,
or additional watched inputs. Output writes do not trigger another build.

## Readiness, errors, and shutdown

The first client build runs automatically, before WebUI prepares the initial
render. With `--watch`, input changes schedule serial client builds followed
by the necessary WebUI work. Changes during an active build are coalesced into
a follow-up. Superseded builds do not publish or reload; one current successful
build produces one SSE reload.

While a build is pending, app, template, and asset requests return 503 with
`Cache-Control: no-store`. A build failure returns 500 and does not reload.
Editing an input triggers recovery. The live-reload connection and `/api/*`
forwarding remain available. A listening URL is not application readiness:
wait for a successful app response. Already-started requests can finish using
their captured render state; this is not a production asset transaction.
`--format json` retains diagnostic-only JSON output, not a lifecycle protocol
or human startup banner. Use a known `--port` for HTTP readiness in that mode.

With `webui dev --no-watch`, or `webui serve` without `--watch`, the initial
build runs once and no watcher or live-reload client is installed.
`WEBUI_NO_WATCH` likewise disables watching. An initial build failure in this
mode exits nonzero.

`--client-build-timeout-ms` sets the positive timeout for builder startup,
rebuild, and disposal (default `120000`). Worker startup, unexpected exit,
communication failure, and timeout fail explicitly rather than serving stale
success. An idle healthy server has no build deadline.

Stop with Ctrl+C. If a host spawns the CLI with piped stdin, keep that pipe open
for the server's lifetime and close it to request an orderly stop. Stdin has no
command protocol. WebUI stops serving, calls `dispose`, and exits; disposal
failures produce a nonzero exit. Hosts should await process exit, keep
diagnostics visible, and report an unexpected exit rather than start a
substitute server.

## Theme, state, and production

Configured `--theme`, `--state`, and projection inputs may be generated beneath
`--servedir` by the first rebuild. Their parent directories must already exist.
WebUI consumes current successful outputs, not values from an earlier build.
SDK theme resolution and token merging remain unchanged. With `--api-port`,
request-derived state is still fetched per request, preserving encoded paths
and queries. See the [CLI reference](./) for headers, nonce-aware CSP, and
`--api-state-errors strict`.

For production, complete a production client build and then run the existing
`webui build --entry ... --out ...` command. This option is for development,
not an additional production bundling or deployment API. Check
`webui dev --help` and `webui serve --help` for availability in the native
executable you are running.
