# microsoft-webui-protocol

Protobuf protocol definitions and serialization for the [WebUI](https://github.com/microsoft/webui) framework. Defines the binary format that carries compiled template data from the build step to the renderer.

## Overview

`microsoft-webui-protocol` uses `prost` for zero-copy protobuf encoding and decoding. It defines the `WebUIProtocol` message and all fragment types that flow between the parser and handler.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT — Copyright (c) Microsoft Corporation.
