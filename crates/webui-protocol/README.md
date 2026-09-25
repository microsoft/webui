# microsoft-webui-protocol

Protobuf protocol definitions and serialization for the [WebUI](https://github.com/microsoft/webui) framework. Defines the binary format that carries compiled template data from the build step to the renderer.

## Overview

`microsoft-webui-protocol` uses `prost` for zero-copy protobuf encoding and decoding. It defines the `WebUIProtocol` message and all fragment types that flow between the parser and handler.

## Features

- `projection-manifest` - Enables the build-time state-projection manifest schema and validation APIs. This is disabled by default so handler-only consumers do not compile the SHA-256 implementation.
- `regenerate-proto` - Regenerates the checked-in Rust protocol types from `proto/webui.proto`.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT — Copyright (c) Microsoft Corporation.
