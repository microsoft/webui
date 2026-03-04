# WebUI FFI Handler

The WebUI FFI (Foreign Function Interface) handler exposes the rendering pipeline as a C-compatible shared library. Any language with C interop — Go, C#, Python, Ruby, PHP, and more — can load the library and render WebUI templates without a JavaScript runtime.

## Building the Shared Library

```bash
cargo build -p webui-ffi            # debug
cargo build -p webui-ffi --release  # release
```

This produces a shared library:

| Platform | Library file |
|---|---|
| macOS | `target/release/libwebui_ffi.dylib` |
| Linux | `target/release/libwebui_ffi.so` |
| Windows | `target/release/webui_ffi.dll` |

The generated C header is at `crates/webui-ffi/include/webui_ffi.h`.

## Two Rendering Modes

### 1. One-Shot: `webui_render`

Parse and render in a single call. Best for simple use cases where you pass raw HTML templates.

```c
char *html = webui_render(
    "<h1>{{title}}</h1><ul><for each=\"item in items\"><li>{{item}}</li></for></ul>",
    "{\"title\": \"Groceries\", \"items\": [\"Milk\", \"Eggs\"]}"
);
if (html == NULL) {
    printf("Error: %s\n", webui_last_error());
} else {
    printf("%s\n", html);
    webui_free(html);
}
```

### 2. Pre-Compiled: `webui_handler_create` + `webui_handler_render`

Create a reusable handler and render pre-compiled protobuf protocols. Best for production use where the protocol is built once with `webui build` and rendered many times.

```c
// Create handler (optionally with a plugin)
void *handler = webui_handler_create();
// or: void *handler = webui_handler_create_with_plugin("fast");

// Load protocol.bin from disk (your code)
uint8_t *data = load_file("dist/protocol.bin", &len);

// Render
char *html = webui_handler_render(handler, data, len, state_json);
if (html) {
    // use html...
    webui_free(html);
}

// Clean up
webui_handler_destroy(handler);
```

## C API Reference

| Function | Signature | Description |
|----------|-----------|-------------|
| `webui_render` | `char *(const char *html, const char *data_json)` | Parse + render in one call |
| `webui_handler_create` | `void *()` | Create a reusable handler (no plugin) |
| `webui_handler_create_with_plugin` | `void *(const char *plugin_id)` | Create a handler with a named plugin (e.g., `"fast"`) |
| `webui_handler_render` | `char *(void *handler, const uint8_t *data, uintptr_t len, const char *json)` | Render a pre-compiled protocol |
| `webui_handler_destroy` | `void(void *handler)` | Destroy a handler instance |
| `webui_free` | `void(char *ptr)` | Free a string returned by any render function |
| `webui_last_error` | `const char *()` | Get per-thread error message |

## Error Handling

The FFI uses thread-local error storage following the POSIX `dlerror()` pattern:

1. Any function that can fail returns `NULL` on error
2. Call `webui_last_error()` immediately after to get a human-readable message
3. The error pointer is valid until the next FFI call on the same thread
4. Each thread has independent error state — safe for concurrent use

```c
char *result = webui_render(html, json);
if (result == NULL) {
    const char *err = webui_last_error();  // valid until next FFI call
    fprintf(stderr, "Render failed: %s\n", err);
    // do NOT free err
}
```

## Memory Management

| Pointer source | Who frees it? | How? |
|---|---|---|
| `webui_render` return | Caller | `webui_free(ptr)` |
| `webui_handler_render` return | Caller | `webui_free(ptr)` |
| `webui_last_error` return | Library (do **not** free) | Replaced on next call |
| `webui_handler_create` return | Caller | `webui_handler_destroy(ptr)` |
| `webui_handler_create_with_plugin` return | Caller | `webui_handler_destroy(ptr)` |

## Using Plugins

Pass a plugin identifier string to `webui_handler_create_with_plugin`:

```c
// Create handler with FAST-HTML hydration plugin
void *handler = webui_handler_create_with_plugin("fast");
if (handler == NULL) {
    printf("Error: %s\n", webui_last_error());
    return 1;
}

// Render — output includes hydration markers
char *html = webui_handler_render(handler, protocol_data, protocol_len, state_json);

webui_free(html);
webui_handler_destroy(handler);
```

Currently supported plugins: `"fast"`. Pass `NULL` for no plugin (equivalent to `webui_handler_create`).

See [Plugins](/guide/concepts/plugins/) for details on what the FAST plugin injects.

## Language Examples

For complete examples in Python, Go, and C#, see the [Language Integrations](/guide/integrations) page.

## Next Steps

- [Language Integrations](/guide/integrations) — Python, Go, C# examples with full code
- [Plugins](/guide/concepts/plugins/) — Plugin system and FAST-HTML hydration
- [CLI Reference](/guide/cli/) — Building protocols with `webui build`
