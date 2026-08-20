# microsoft-webui-node

Node.js native addon for the [WebUI](https://github.com/microsoft/webui) framework, built with [napi-rs](https://napi.rs).

## Overview

`microsoft-webui-node` compiles to a platform-specific `.node` addon that exposes WebUI's rendering API to Node.js hosts (e.g. Express, Fastify) without spawning a subprocess.

## Host-driven streaming

`Protocol.streamResponse(entry, requestPath, options)` returns a
`StreamingSession`. Call `start(stateJson)` once, then call
`resume(instanceId, stateJson, mode)` for each discovered boundary. State and
patch arguments are JSON strings. Both step methods return:

```js
{
  bytes: Buffer,
  done: boolean,
  boundary?: { instanceId, declarationId, owner, name, key }
}
```

`mode` is `"final"` (the default) or `"updatable"`. A committed updatable
occurrence accepts `update(instanceId, patch)`, which returns a `Buffer`.
Boundary keys preserve JSON identity: string keys are JavaScript strings and
finite numeric keys are JavaScript numbers. Boundaries are discovered through
entries, reusable components, runtime branches, loops, and selected routes.
The step with `done: true` already contains terminal and document-tail bytes.

## Benchmark

The runtime benchmark in
[`examples/integration/node-addon-bench`](../../examples/integration/node-addon-bench)
measures protocol construction, buffered rendering, and streaming callbacks across
the real V8/N-API boundary:

```bash
cargo xtask bench node-addon
```

It supports the repository-wide `--save-baseline NAME` and `--baseline NAME`
workflow documented in [`BENCHMARKS.md`](../../BENCHMARKS.md).

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT — Copyright (c) Microsoft Corporation.
