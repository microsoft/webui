# WebUI Rust Example

Minimal example showing how to use WebUI as a Rust library to render a pre-built protocol with state data.

## Prerequisites

Build the hello-world app first:

```bash
cargo run -p microsoft-webui-cli -- build ../../app/hello-world/templates --out ../../app/hello-world/dist
```

## Usage

```bash
cargo run -- ../../app/hello-world/dist/protocol.bin ../../app/hello-world/data/state.json
```

This loads `protocol.bin`, passes the state from `state.json`, and prints the rendered HTML to stdout.

For a protocol containing runtime `<boundary>` declarations, drive the clean
cursor API directly:

```bash
cargo run -- streaming-protocol.bin state.json --plugin=webui --streaming
```

The example calls `StreamingResponse::start()`, then resumes each
`BoundaryDescriptor::instance_id` returned by the preceding step. The final
resume emits the terminal automatically when `StreamStatus::done` becomes true;
there is no separate finish call. Real servers can commit an occurrence as
`BoundaryMode::Updatable` and call `update(instance_id, patch)` before the final
step.
