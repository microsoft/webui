# microsoft-webui-ffi

C-compatible FFI boundary for the [WebUI](https://github.com/microsoft/webui) framework. Exposes the WebUI renderer to any host language via a stable C ABI.

## Overview

`microsoft-webui-ffi` compiles to a `cdylib` (`libwebui_ffi.so` / `webui_ffi.dll` / `libwebui_ffi.dylib`) that host language bindings (e.g. .NET, Node.js) load at runtime. The generated C header (`webui_ffi.h`) describes the full public API.

Production hosts should call `webui_protocol_create` once when loading
`protocol.bin`, then pass that handle to the render, partial,
component-template, and token functions. This avoids protobuf decoding and
deterministic index construction on every request. Release the shared handle with
`webui_protocol_destroy`.

A full render is returned either as a NUL-terminated string from
`webui_handler_render` (free with `webui_free`) or as an owned document from
`webui_handler_render_result`, whose bytes are borrowed with
`webui_render_result_bytes` until `webui_render_result_destroy`. Prefer the
document handle when the host tracks lengths; it avoids a NUL scan and copy of
large documents.

## Progressive streaming

Create one `webui_streaming_session_t` per response. Drive it with
`webui_streaming_session_start`, `webui_streaming_session_resume`, and
`webui_streaming_session_advance`, plus
`webui_streaming_session_update`. Start, resume, and advance return an opaque
`webui_streaming_step_t` with accessors for binary-safe bytes, `done`, and an
optional runtime descriptor:

```text
{ instance_id, declaration_id, owner, name, key }
```

If the step has a descriptor, call `webui_streaming_session_resume`. If it has
neither a descriptor nor `done`, call `webui_streaming_session_advance`. If
`done` is true, the response is complete. Resume must target the currently
pending instance and returns only that occurrence through its checkpoint.
Advance returns the following parent or tail bytes through the next descriptor
or terminal, so no sibling boundary workaround is needed.

Update targets a previously committed updatable instance, is valid between its
resume and advance, and emits state only, never markup. Release step storage
with `webui_streaming_step_destroy`; its byte and descriptor string pointers are
borrowed until that call. Free update byte buffers with `webui_free`.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and integration examples.

## License

MIT - Copyright (c) Microsoft Corporation.
