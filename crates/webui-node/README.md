# microsoft-webui-node

Node.js native addon for the [WebUI](https://github.com/microsoft/webui) framework, built with [napi-rs](https://napi.rs).

## Overview

`microsoft-webui-node` compiles to a platform-specific `.node` addon that exposes WebUI's rendering API to Node.js hosts (e.g. Express, Fastify) without spawning a subprocess.

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
