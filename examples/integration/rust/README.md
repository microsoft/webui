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

The example calls `StreamingResponse::start()`, then follows the step state
exactly: a descriptor requires `resume`, no descriptor with `done == false`
requires `advance`, and `done == true` completes the response. `resume` emits
only that boundary through its checkpoint. `advance` emits the following parent
or tail bytes through the next descriptor or terminal, so no sibling boundary
is needed. The final `advance` emits the terminal; there is no separate finish
call. Real servers can commit an occurrence as `BoundaryMode::Updatable` and
call `update(instance_id, patch)` between its `resume` and `advance`.
