# microsoft-webui-ffi

C-compatible FFI boundary for the [WebUI](https://github.com/microsoft/webui) framework. Exposes the WebUI renderer to any host language via a stable C ABI.

## Overview

`microsoft-webui-ffi` compiles to a `cdylib` (`libwebui_ffi.so` / `webui_ffi.dll` / `libwebui_ffi.dylib`) that host language bindings (e.g. .NET, Node.js) load at runtime. The generated C header (`webui_ffi.h`) describes the full public API.

Production hosts should call `webui_protocol_create` once when loading
`protocol.bin`, then pass that handle to the render, partial,
component-template, and token functions. This avoids protobuf decoding and
deterministic index construction on every request. Release the shared handle with
`webui_protocol_destroy`.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and integration examples.

## License

MIT - Copyright (c) Microsoft Corporation.
