# C API

The WebUI FFI (Foreign Function Interface) handler exposes the rendering pipeline as a C-compatible shared library. Any language with C interop, Go, Python, Ruby, PHP, Lua, and more, can load the library and render WebUI templates without a JavaScript runtime. .NET applications should prefer the managed `Microsoft.WebUI` NuGet package, which restores native runtime packages transitively.

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

## Rendering Modes

### One-shot: `webui_render`

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

### Raw protocol: `webui_handler_create` + `webui_handler_render`

Create a reusable handler and render pre-compiled protobuf bytes. This
compatibility path decodes the protocol on each render.

### Prepared protocol: recommended for repeated rendering

Decode and index `protocol.bin` once with `webui_protocol_create`, then use the
prepared entry points:

```c
void *handler = webui_handler_create_with_plugin("webui");
webui_handler_set_nonce(handler, "Ep7tTOr+HyRkByAPXxZ9ag==");

uint8_t *data = load_file("dist/protocol.bin", &len);
void *protocol = webui_protocol_create(data, len);
if (protocol == NULL) {
    fprintf(stderr, "Protocol error: %s\n", webui_last_error());
    webui_handler_destroy(handler);
    return;
}

char *html = webui_handler_render_prepared(
    handler, protocol, state_json, "index.html", request_path
);
if (html) {
    webui_free(html);
}

webui_protocol_destroy(protocol);
webui_handler_destroy(handler);
```

Prepared protocol handles are thread-safe. Handler instances are safe for
concurrent renders as long as configuration such as the nonce is not mutated
concurrently.

## C API Reference

The generated C header is at `crates/webui-ffi/include/webui_ffi.h`.

### webui_render

```c
char *webui_render(const char *html, const char *data_json);
```

Parse an HTML template and render it with JSON state data in a single call.
Use it for prototypes and one-shot templates. Production servers should build a
protocol ahead of time and reuse a prepared protocol handle.

- `html`, null-terminated UTF-8 string containing the HTML template.
- `data_json`, null-terminated UTF-8 JSON string with the render state.
- **Returns** a heap-allocated null-terminated UTF-8 string with the rendered HTML, or `NULL` on error.
- The caller **must** free the returned string with `webui_free()`.

### webui_free

```c
void webui_free(char *string_ptr);
```

Free a string returned by `webui_render` or `webui_handler_render`. Passing `NULL` is a safe no-op.

### webui_last_error

```c
const char *webui_last_error();
```

Return the last error message for the current thread, or `NULL` if no error has occurred. Call this after any function returns `NULL` to get a human-readable diagnostic.

- The returned pointer is **owned by the library**. Do **not** free it.
- The pointer is valid until the next FFI call on the same thread.
- Each thread has its own independent error state.

### webui_handler_create

```c
void *webui_handler_create();
```

Create a reusable handler instance. Returns an opaque pointer that must eventually be freed with `webui_handler_destroy`. Use this with `webui_handler_render` when rendering pre-compiled protobuf protocols.

### webui_handler_create_with_plugin

```c
void *webui_handler_create_with_plugin(const char *plugin_id);
```

Create a reusable handler instance with a named plugin. Pass `NULL` for no plugin (equivalent to `webui_handler_create`). See [Plugins](/guide/concepts/plugins/) for the available identifiers.

- `plugin_id`, null-terminated UTF-8 string identifying the plugin, or `NULL`.
- **Returns** an opaque pointer on success, or `NULL` on error (call `webui_last_error()` for details).
- The caller **must** free the returned pointer with `webui_handler_destroy()`.

### webui_handler_destroy

```c
void webui_handler_destroy(void *handler_ptr);
```

Destroy a handler instance created by `webui_handler_create`. Passing `NULL` is a safe no-op.

### webui_handler_set_nonce

```c
void webui_handler_set_nonce(void *handler_ptr, const char *nonce);
```

Set the CSP nonce for inline tags on a handler instance. When set, all subsequent renders include `nonce="VALUE"` on every inline `<script>` tag emitted during SSR (including the `<script type="importmap">` tags that register Module-strategy CSS), and emit a `<meta name="webui-nonce" content="VALUE">` tag in the `<head>`.

- `handler_ptr`, pointer returned by `webui_handler_create`.
- `nonce`, null-terminated UTF-8 string (typically a base64-encoded random value), or `NULL` to clear a previously set nonce.

The nonce is written verbatim — pass the raw base64 string without any encoding. The same value should appear in your `Content-Security-Policy` header.

::: warning Thread Safety
Concurrent render calls are supported after configuration. Do not call
`webui_handler_set_nonce` or `webui_handler_destroy` while another operation is
using the same handler.
:::

### webui_protocol_create / webui_protocol_destroy

```c
void *webui_protocol_create(const uint8_t *protocol_data,
                            uintptr_t protocol_len);
void webui_protocol_destroy(void *protocol_ptr);
```

Decode protobuf bytes and build reusable component and route indices. The
returned handle is thread-safe and can be shared across requests. Destroy it
after every render using it has completed. Passing `NULL` to
`webui_protocol_destroy` is a safe no-op.

### webui_handler_render

```c
char *webui_handler_render(void *handler_ptr,
                           const uint8_t *protocol_data,
                           uintptr_t protocol_len,
                           const char *data_json,
                           const char *entry_id,
                           const char *request_path);
```

Render a pre-compiled WebUI protocol (protobuf binary) with JSON state data. This is the lower-level API for callers that have already compiled their templates to protobuf via the CLI.

- `handler_ptr`, pointer returned by `webui_handler_create`.
- `protocol_data`, pointer to protobuf binary data.
- `protocol_len`, length of the protobuf data in bytes.
- `data_json`, null-terminated UTF-8 JSON string with the render state.
- `entry_id`, null-terminated UTF-8 string identifying the entry fragment (e.g., `"index.html"`).
- `request_path`, null-terminated UTF-8 string with the request path for route matching (e.g., `"/users/42"`).
- **Returns** a heap-allocated string on success, or `NULL` on error.
- The caller **must** free the returned string with `webui_free()`.

This function decodes `protocol_data` on every call. Use
`webui_handler_render_prepared` for a protocol rendered repeatedly.

### webui_handler_render_prepared

```c
char *webui_handler_render_prepared(void *handler_ptr,
                                    const void *protocol_ptr,
                                    const char *data_json,
                                    const char *entry_id,
                                    const char *request_path);
```

Render with a handle from `webui_protocol_create`. Output, errors, and string
ownership match `webui_handler_render`.

### Partial, component-template, and token helpers

| Function | Result |
|----------|--------|
| `webui_render_partial(...)` | Complete JSON partial response containing active-route projected `state`, templates, inventory, path, and route chain |
| `webui_render_partial_prepared(...)` | Same projected response using a prepared protocol |
| `webui_render_component_templates(...)` | Requested component template payloads and updated inventory |
| `webui_render_component_templates_prepared(...)` | Same query using a prepared protocol |
| `webui_protocol_tokens(...)` | Newline-delimited CSS token names |
| `webui_protocol_tokens_prepared(...)` | Token names using a prepared protocol |

The partial functions validate `state_json`, skip unselected values without
materializing them, and copy only raw values required by authored components on
the active route.

## Error Handling

The FFI uses thread-local error storage following the POSIX `dlerror()` pattern:

1. Any function that can fail returns `NULL` on error.
2. Call `webui_last_error()` immediately after to get a human-readable message.
3. The error pointer is valid until the next FFI call on the same thread.
4. Each thread has independent error state, safe for concurrent use.

```c
char *result = webui_render(html, json);
if (result == NULL) {
    const char *err = webui_last_error();  // valid until next FFI call
    fprintf(stderr, "Render failed: %s\n", err);
    // do NOT free err
}
```

## Memory Management

Two rules to remember:

1. **Free what you receive.** Every non-`NULL` string returned by `webui_render` or `webui_handler_render` is heap-allocated. You must free it with `webui_free()`.
2. **Don't free error strings.** The pointer from `webui_last_error()` is owned by the library. It remains valid until your next FFI call on the same thread.

| Pointer source | Who frees it? | How? |
|---|---|---|
| `webui_render` | Caller | `webui_free(ptr)` |
| `webui_handler_render` | Caller | `webui_free(ptr)` |
| `webui_handler_render_prepared` | Caller | `webui_free(ptr)` |
| Partial, component-template, and token strings | Caller | `webui_free(ptr)` |
| `webui_last_error` | Library (do **not** free) | Replaced on next call |
| `webui_handler_create` | Caller | `webui_handler_destroy(ptr)` |
| `webui_handler_create_with_plugin` | Caller | `webui_handler_destroy(ptr)` |
| `webui_protocol_create` | Caller | `webui_protocol_destroy(ptr)` |

## Using Plugins

Pass a plugin identifier string to `webui_handler_create_with_plugin`:

```c
// Create handler with a hydration plugin
void *handler = webui_handler_create_with_plugin("webui");
if (handler == NULL) {
    printf("Error: %s\n", webui_last_error());
    return 1;
}

// Render, output includes hydration markers
char *html = webui_handler_render(handler, protocol_data, protocol_len,
                                  state_json, "index.html", "/");

webui_free(html);
webui_handler_destroy(handler);
```

Pass `NULL` for no plugin (equivalent to `webui_handler_create`). See [Plugins](/guide/concepts/plugins/) for the available identifiers.

## Python

Python's built-in `ctypes` module can load the shared library directly. No pip packages needed.

```python
import ctypes
from ctypes import c_char_p, c_void_p

# Load the library
lib = ctypes.cdll.LoadLibrary("./target/debug/libwebui_ffi.dylib")  # or .so / .dll

# Declare function signatures
lib.webui_render.argtypes = [c_char_p, c_char_p]
lib.webui_render.restype = c_void_p

lib.webui_free.argtypes = [c_void_p]
lib.webui_free.restype = None

lib.webui_last_error.argtypes = []
lib.webui_last_error.restype = c_char_p

# Render a template
html = b'<h1>{{title}}</h1><ul><for each="item in items"><li>{{item}}</li></for></ul>'
state = b'{"title": "Groceries", "items": ["Milk", "Eggs", "Bread"]}'

ptr = lib.webui_render(html, state)

if ptr is None or ptr == 0:
    print("Error:", lib.webui_last_error().decode("utf-8"))
else:
    result = ctypes.cast(ptr, c_char_p).value.decode("utf-8")
    lib.webui_free(ptr)
    print(result)
    # Output: <h1>Groceries</h1><ul><li>Milk</li><li>Eggs</li><li>Bread</li></ul>
```

> **Why `c_void_p`?** Using `c_void_p` as the return type instead of `c_char_p` prevents `ctypes` from automatically converting the pointer to a Python `bytes` object. This lets you copy the string first, then explicitly free the original pointer with `webui_free()`.

## Go

Go's `cgo` lets you call C functions directly. Link against `libwebui_ffi` and use C strings with standard lifecycle management.

```go
package main

// #cgo LDFLAGS: -L./target/debug -lwebui_ffi
// #include <stdlib.h>
//
// extern char       *webui_render(const char *html, const char *data_json);
// extern void        webui_free(char *ptr);
// extern const char *webui_last_error();
import "C"
import (
	"fmt"
	"unsafe"
)

func render(html, dataJSON string) (string, error) {
	cHTML := C.CString(html)
	defer C.free(unsafe.Pointer(cHTML))

	cJSON := C.CString(dataJSON)
	defer C.free(unsafe.Pointer(cJSON))

	ptr := C.webui_render(cHTML, cJSON)
	if ptr == nil {
		return "", fmt.Errorf("render failed: %s", C.GoString(C.webui_last_error()))
	}
	defer C.webui_free(ptr)

	return C.GoString(ptr), nil
}

func main() {
	html := `<h1>{{title}}</h1><ul><for each="item in items"><li>{{item}}</li></for></ul>`
	state := `{"title": "Groceries", "items": ["Milk", "Eggs", "Bread"]}`

	result, err := render(html, state)
	if err != nil {
		fmt.Println(err)
		return
	}
	fmt.Println(result)
	// Output: <h1>Groceries</h1><ul><li>Milk</li><li>Eggs</li><li>Bread</li></ul>
}
```

> **Memory note:** `C.GoString(ptr)` copies the string into Go-managed memory, so it's safe to call `webui_free` immediately after.

## C\#

For most .NET applications, prefer the managed `Microsoft.WebUI` NuGet package. The P/Invoke pattern below documents the underlying ABI for custom bindings or manual native loading.

Use `DllImport` (P/Invoke) to call the C API. Strings going *in* can be marshalled automatically with `LPUTF8Str`; strings coming *out* require manual marshalling via `IntPtr` to control when the native memory is freed.

```csharp
using System;
using System.Runtime.InteropServices;

class WebUI
{
    [DllImport("webui_ffi")]
    static extern IntPtr webui_render(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string html,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string dataJson);

    [DllImport("webui_ffi")]
    static extern void webui_free(IntPtr ptr);

    [DllImport("webui_ffi")]
    static extern IntPtr webui_last_error();

    static string Render(string html, string dataJson)
    {
        IntPtr ptr = webui_render(html, dataJson);
        if (ptr == IntPtr.Zero)
        {
            string err = Marshal.PtrToStringUTF8(webui_last_error()) ?? "unknown error";
            throw new InvalidOperationException($"Render failed: {err}");
        }

        string result = Marshal.PtrToStringUTF8(ptr) ?? "";
        webui_free(ptr);
        return result;
    }

    static void Main()
    {
        string html = @"<h1>{{title}}</h1>
            <ul><for each=""item in items""><li>{{item}}</li></for></ul>";
        string state = @"{""title"": ""Groceries"", ""items"": [""Milk"", ""Eggs"", ""Bread""]}";

        Console.WriteLine(Render(html, state));
        // Output: <h1>Groceries</h1><ul><li>Milk</li><li>Eggs</li><li>Bread</li></ul>
    }
}
```

> **Why `IntPtr` for return values?** If you use `string` as the return type, the .NET marshaller will try to free the memory with `CoTaskMemFree`, which will crash since the string was allocated by Rust. Always receive as `IntPtr`, copy with `Marshal.PtrToStringUTF8`, and free with `webui_free`.

## Other Languages

Any language with C FFI support can use WebUI. The pattern is always the same:

1. Load the shared library (`libwebui_ffi.dylib` / `.so` / `.dll`).
2. Declare the functions you need. For a server, prefer
   `webui_protocol_create`, `webui_handler_render_prepared`,
   `webui_protocol_destroy`, `webui_free`, and `webui_last_error`.
3. Pass UTF-8 null-terminated strings for `html` and `data_json`.
4. Check the return value, `NULL` means an error occurred.
5. Copy the returned string into your language's managed memory, then call `webui_free`.

## Next Steps

- [Plugins](/guide/concepts/plugins/), Plugin system and built-in plugin reference
- [CLI Reference](/guide/cli/), Building protocols with `webui build`
