---
title: "C API and FFI"
description: "Build the native shared library, render through opaque handles, and manage ownership, errors, and streaming from C or other hosts."
---

# C API

Build WebUI as a native shared library to call the Rust renderer through C
interop. Load a protocol handle once, reuse it for rendering, and release each
result with its designated function. Your host must support the target's
native library and ABI; this does not require a JavaScript SSR engine.

Prefer the managed [Microsoft.WebUI package](./dotnet) for .NET or the native
[Python package](./python) on its supported targets. The Go and `ctypes`
examples below demonstrate direct FFI ownership rather than additional
first-class packages.

## Building the Shared Library

```bash
cargo build -p microsoft-webui-ffi            # debug
cargo build -p microsoft-webui-ffi --release  # release
```

This produces a shared library:

| Platform | Library file |
|---|---|
| macOS | `target/release/libwebui_ffi.dylib` |
| Linux | `target/release/libwebui_ffi.so` |
| Windows | `target/release/webui_ffi.dll` |

The generated C header is at `crates/webui-ffi/include/webui_ffi.h`.

## Protocol Rendering

Decode and index `protocol.bin` once with `webui_protocol_create`, then use the
protocol handle for every operation:

```c
void *handler = webui_handler_create_with_plugin("webui");
webui_handler_set_nonce(handler, "Ep7tTOr+HyRkByAPXxZ9ag==");

uint8_t *data = load_file("dist/protocol.bin", &len);
webui_protocol_t *protocol = webui_protocol_create(data, len);
if (protocol == NULL) {
    fprintf(stderr, "Protocol error: %s\n", webui_last_error());
    webui_handler_destroy(handler);
    return;
}

webui_render_t *result = webui_handler_render(
    handler, protocol, state_json, "index.html", request_path
);
if (result != NULL) {
    uintptr_t html_len = 0;
    const uint8_t *html = webui_render_bytes(result, &html_len);
    if (html != NULL) {
        fwrite(html, 1, html_len, stdout);
    }
    webui_render_free(result);
}

webui_protocol_destroy(protocol);
webui_handler_destroy(handler);
```

Loaded protocol handles are thread-safe. Handler instances are safe for
concurrent renders as long as configuration such as the nonce is not mutated
concurrently.

## C API Reference

The generated C header is at `crates/webui-ffi/include/webui_ffi.h`.

### webui_free

```c
void webui_free(char *string_ptr);
```

Free a string returned by a WebUI protocol operation such as
`webui_protocol_render_partial`. Passing `NULL` is a safe no-op.

Never pass an opaque handle to `webui_free`: release a document from
`webui_handler_render` with `webui_render_free`, and streaming
steps and sessions with their own destroy functions. C converts these `void *`
handles to `char *` silently, so the compiler will not catch the mistake.

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

Create a reusable handler instance with a named plugin. Pass `NULL` for no plugin (equivalent to `webui_handler_create`). See [Plugins](/reference/plugins/) for the available identifiers.

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

**Thread safety.** Concurrent render calls are supported after configuration. Do not call
`webui_handler_set_nonce` or `webui_handler_destroy` while another operation is
using the same handler.

### Reserved `$webui` state channel

A top-level `$webui` object in the render state JSON passed to
`webui_handler_render` or a streaming session
may carry `headEnd`, `bodyStart`, and `bodyEnd` strings, each emitted **raw**
at the matching structural boundary (before `</head>`, after `<body>`, before
`</body>`):

```json
{"$webui": {"headEnd": "<meta name=\"x\">", "bodyEnd": "<script src=\"/a.js\"></script>"}}
```

Members that are missing, `null`, empty, or not strings are ignored.
The `$webui` object is excluded from client state, but its HTML is emitted
into the document. No extra API call is needed.

**Safety.** The values are written verbatim with no escaping, exactly like the
Rust `head_inject` / `body_inject` options. Never let untrusted request input
reach the `$webui` key.

### webui_protocol_create / webui_protocol_destroy

```c
typedef void webui_protocol_t;

webui_protocol_t *webui_protocol_create(const uint8_t *protocol_data,
                                        uintptr_t protocol_len);
void webui_protocol_destroy(webui_protocol_t *protocol_ptr);
```

Decode protobuf bytes and build reusable component and route indices. The
returned handle is thread-safe and can be shared across requests. Destroy it
after every render using it has completed. Passing `NULL` to
`webui_protocol_destroy` is a safe no-op.

### webui_handler_render

```c
webui_render_t *webui_handler_render(void *handler_ptr,
                                            const webui_protocol_t *protocol_ptr,
                                            const char *data_json,
                                            const char *entry_id,
                                            const char *request_path);
```

Render a protocol handle created by `webui_protocol_create` with JSON state data.

- `handler_ptr`, pointer returned by `webui_handler_create`.
- `protocol_ptr`, pointer returned by `webui_protocol_create`.
- `data_json`, null-terminated UTF-8 JSON string with the render state.
- `entry_id`, null-terminated UTF-8 string identifying the entry fragment (e.g., `"index.html"`).
- `request_path`, null-terminated UTF-8 string with the request path for route matching (e.g., `"/users/42"`).
- **Returns** an owned document handle on success, or `NULL` on error.
- The caller **must** release the handle with `webui_render_free()`.

### webui_render_bytes / webui_render_free

```c
const uint8_t *webui_render_bytes(const webui_render_t *result_ptr,
                                         uintptr_t *out_len);
void webui_render_free(webui_render_t *result_ptr);
```

`webui_render_bytes` borrows the UTF-8 document returned by
`webui_handler_render` and writes its byte length to a non-null `out_len`.
The bytes are not NUL-terminated, may contain interior NUL bytes, and stay valid
until `webui_render_free`. The accessor returns `NULL` on error.
Passing
`NULL` to `webui_render_free` is a safe no-op.

Use the explicit byte length, not `strlen` or a string-pointer conversion.
A host can keep the handle alive as the backing store of its response and
destroy it once the response has been sent. The handle owns its document, so
it stays valid after the handler or protocol is destroyed.

```c
webui_render_t *result = webui_handler_render(
    handler, protocol, json, "index.html", request_path);
if (result != NULL) {
    uintptr_t html_len = 0;
    const uint8_t *html = webui_render_bytes(result, &html_len);
    if (html != NULL) {
        send_all(socket, html, html_len);
    } else {
        fprintf(stderr, "Result error: %s\n", webui_last_error());
    }
    webui_render_free(result);
}
```

### Partial, component-template, and token helpers

| Function | Result |
|----------|--------|
| `webui_protocol_render_partial(...)` | Complete JSON partial response containing active-route projected `state`, templates, inventory, path, and route chain |
| `webui_protocol_render_component_templates(...)` | Requested component template payloads and updated inventory |
| `webui_protocol_tokens(...)` | Newline-delimited CSS token names |

These functions all accept a protocol handle from `webui_protocol_create`.
The partial function validates `state_json`, skips unselected values without
materializing them, and copies only raw values required by authored components on
the active route.

Keep the loaded handle alive while renders use it. Do not substitute a raw
protocol-byte pointer for a handle.

### Progressive streaming sessions

A streaming session lets a C host render one response in chunks it writes
itself. Start, resume, and advance return owned step handles with borrowed byte
slices; update returns an owned byte buffer. WebUI never touches your socket,
so backpressure and cancellation stay yours.

```c
webui_streaming_session_t *session = webui_streaming_session_create(
    handler, protocol, "index.html", "/");

webui_streaming_step_t *step =
    webui_streaming_session_start(session, initial_state_json);
if (step == NULL) {
    fprintf(stderr, "%s\n", webui_last_error());
}

while (step != NULL) {
    uintptr_t bytes_len = 0;
    const uint8_t *bytes = webui_streaming_step_bytes(step, &bytes_len);
    send_all(socket, bytes, bytes_len);
    if (webui_streaming_step_done(step)) {
        webui_streaming_step_destroy(step);
        break;
    }

    if (webui_streaming_step_has_boundary(step)) {
        /* Copies owner, name, typed key, and IDs from the step accessors. */
        struct app_boundary target = copy_boundary_descriptor(step);
        webui_streaming_step_destroy(step);

        const char *state_json = load_state(&target);
        step = webui_streaming_session_resume(
            session,
            target.instance_id,
            state_json,
            WEBUI_BOUNDARY_MODE_FINAL);
        free_boundary_descriptor(&target);
    } else {
        webui_streaming_step_destroy(step);
        step = webui_streaming_session_advance(session);
    }
    if (step == NULL) {
        fprintf(stderr, "%s\n", webui_last_error());
        break;
    }
}

webui_streaming_session_destroy(session);
```

| Function | Result |
|----------|--------|
| `webui_streaming_session_create(handler, protocol, entry_id, request_path)` | Session handle, or `NULL`. Inherits the handler's nonce (set with `webui_handler_set_nonce`); head/body injection travels through the reserved `$webui` state key on `state_json`, not through this call. |
| `webui_streaming_session_destroy(session)` | Releases the session. `NULL` is a safe no-op. |
| `webui_streaming_session_start(session, state_json)` | Owned step through the first runtime occurrence or terminal, or `NULL` |
| `webui_streaming_session_resume(session, instance_id, state_json, mode)` | Owned step containing only the pending occurrence through its checkpoint |
| `webui_streaming_session_advance(session)` | Owned step containing following parent bytes through the next occurrence or terminal |
| `webui_streaming_session_update(session, instance_id, patch_json, out_len)` | Projected state bytes for a committed updatable occurrence |
| `webui_streaming_step_bytes(step, out_len)` | Borrow binary-safe step bytes until destroy |
| `webui_streaming_step_done(step)` / `webui_streaming_step_has_boundary(step)` | Read completion and descriptor presence |
| `webui_streaming_step_boundary_*` | Read IDs, owner/name slices, key type, and typed key |
| `webui_streaming_step_destroy(step)` | Release the opaque step and all borrowed pointers |

`webui_streaming_step_t` is opaque. Step bytes, owner, name, and string keys are
borrowed slices with explicit lengths and are not NUL-terminated. Key type is
none, string, or number; numeric keys are returned as `double`. Copy any
descriptor values needed after destroying the step. Free update bytes with
`webui_free`.

If a step has a descriptor, call `resume`. If it has neither a descriptor nor
`done`, call `advance`. If `done` is true, the response is complete. `resume`
is boundary-only so the host can send that checkpoint immediately; `advance`
renders the following parent or document-tail bytes. An update may be emitted
between `resume` and `advance`.

The session clones its own references to the handler and protocol, so you may
destroy them in any order. Drive a session from one thread at a time. See
[Streaming Boundaries](/learn/directives/boundary) for the authoring
and occurrence rules.

## Error Handling

The FFI uses thread-local error storage following the POSIX `dlerror()` pattern:

1. Any function that can fail returns `NULL` on error.
2. Call `webui_last_error()` immediately after to get a human-readable message.
3. The error pointer is valid until the next FFI call on the same thread.
4. Each thread has independent error state, safe for concurrent use.

```c
webui_render_t *result = webui_handler_render(
    handler, protocol, json, "index.html", request_path
);
if (result == NULL) {
    const char *err = webui_last_error();  // valid until next FFI call
    fprintf(stderr, "Render failed: %s\n", err);
    // do NOT free err
}
```

## Memory Management

Two rules to remember:

1. **Free what you receive.** Every non-`NULL` string returned by a render or
   protocol operation is heap-allocated. You must free it with `webui_free()`.
   Opaque handles are released with their own destroy function from the table
   below, never with `webui_free()`.
2. **Don't free error strings.** The pointer from `webui_last_error()` is owned by the library. It remains valid until your next FFI call on the same thread.

| Pointer source | Who frees it? | How? |
|---|---|---|
| `webui_handler_render` handle and borrowed bytes | Caller | `webui_render_free(result)` |
| Partial, component-template, and token strings | Caller | `webui_free(ptr)` |
| Streaming update bytes | Caller | `webui_free(ptr)` |
| Streaming step handle and borrowed fields | Caller | `webui_streaming_step_destroy(step)` |
| `webui_last_error` | Library (do **not** free) | Replaced on next call |
| `webui_handler_create` | Caller | `webui_handler_destroy(ptr)` |
| `webui_handler_create_with_plugin` | Caller | `webui_handler_destroy(ptr)` |
| `webui_protocol_create` | Caller | `webui_protocol_destroy(ptr)` |
| `webui_streaming_session_create` | Caller | `webui_streaming_session_destroy(ptr)` |

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
void *protocol = webui_protocol_create(protocol_data, protocol_len);
webui_render_t *result = webui_handler_render(
    handler, protocol, state_json, "index.html", "/"
);

webui_render_free(result);
webui_protocol_destroy(protocol);
webui_handler_destroy(handler);
```

Pass `NULL` for no plugin (equivalent to `webui_handler_create`). See [Plugins](/reference/plugins/) for the available identifiers.

## Python

Most Python applications should use the `microsoft-webui` package
instead of using this FFI directly — it is a native PyO3 binding (not a
`ctypes` wrapper) with a `Renderer` facade, typed `bytes`/`str` returns, and a
host-driven `StreamingSession`. See [Python](./python) for installation and
examples.

### Advanced fallback: `ctypes`

If no Python wheel covers your environment, `ctypes` is an alternative when
you can build or obtain a compatible native library. It does not establish
support for every Python interpreter or target. No extra pip package is
needed, but you own memory management, argument marshalling, and thread safety.

```python
import ctypes
from pathlib import Path
from ctypes import c_char_p, c_size_t, c_uint8, c_void_p, POINTER

lib = ctypes.cdll.LoadLibrary("./target/debug/libwebui_ffi.dylib")  # or .so / .dll

lib.webui_protocol_create.argtypes = [POINTER(c_uint8), c_size_t]
lib.webui_protocol_create.restype = c_void_p
lib.webui_protocol_destroy.argtypes = [c_void_p]
lib.webui_handler_create.restype = c_void_p
lib.webui_handler_destroy.argtypes = [c_void_p]
lib.webui_handler_render.argtypes = [
    c_void_p, c_void_p, c_char_p, c_char_p, c_char_p
]
lib.webui_handler_render.restype = c_void_p
lib.webui_render_bytes.argtypes = [c_void_p, POINTER(c_size_t)]
lib.webui_render_bytes.restype = POINTER(c_uint8)
lib.webui_render_free.argtypes = [c_void_p]
lib.webui_render_free.restype = None
lib.webui_last_error.restype = c_char_p

protocol_bytes = Path("dist/protocol.bin").read_bytes()
buffer = (c_uint8 * len(protocol_bytes)).from_buffer_copy(protocol_bytes)
protocol = lib.webui_protocol_create(buffer, len(protocol_bytes))
handler = lib.webui_handler_create()

ptr = lib.webui_handler_render(
    handler,
    protocol,
    b'{"title":"Groceries"}',
    b"index.html",
    b"/",
)

if ptr is None or ptr == 0:
    print("Error:", lib.webui_last_error().decode("utf-8"))
else:
    try:
        length = c_size_t()
        data = lib.webui_render_bytes(ptr, ctypes.byref(length))
        if not data:
            raise RuntimeError(lib.webui_last_error().decode("utf-8"))
        print(ctypes.string_at(data, length.value).decode("utf-8"))
    finally:
        lib.webui_render_free(ptr)

lib.webui_protocol_destroy(protocol)
lib.webui_handler_destroy(handler)
```

> **Why `c_void_p`?** The returned pointer is an opaque document handle, not
> a C string. Read it through `webui_render_bytes` and copy exactly the
> returned length before destroying the handle.

## Go

Go's `cgo` lets you call C functions directly. Link against `libwebui_ffi` and use C strings with standard lifecycle management.

```go
package main

// #cgo LDFLAGS: -L./target/debug -lwebui_ffi
// #include <stdlib.h>
// #include "webui_ffi.h"
import "C"
import (
	"fmt"
	"os"
	"runtime"
	"unsafe"
)

func render(protocol *C.webui_protocol_t, handler unsafe.Pointer, dataJSON string) (string, error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	cJSON := C.CString(dataJSON)
	defer C.free(unsafe.Pointer(cJSON))
	cEntry := C.CString("index.html")
	defer C.free(unsafe.Pointer(cEntry))
	cPath := C.CString("/")
	defer C.free(unsafe.Pointer(cPath))

	ptr := C.webui_handler_render(handler, protocol, cJSON, cEntry, cPath)
	if ptr == nil {
		return "", fmt.Errorf("render failed: %s", C.GoString(C.webui_last_error()))
	}
	defer C.webui_render_free(ptr)

	var length C.uintptr_t
	bytes := C.webui_render_bytes(ptr, &length)
	if bytes == nil {
		return "", fmt.Errorf("read result failed: %s", C.GoString(C.webui_last_error()))
	}
	if uint64(length) > uint64(^uint(0)>>1) {
		return "", fmt.Errorf("render result exceeds the Go string size limit")
	}
	return string(unsafe.Slice((*byte)(unsafe.Pointer(bytes)), int(length))), nil
}

func main() {
	bytes, err := os.ReadFile("dist/protocol.bin")
	if err != nil || len(bytes) == 0 {
		panic("protocol.bin is missing or empty")
	}
	protocol := C.webui_protocol_create(
		(*C.uint8_t)(unsafe.Pointer(&bytes[0])),
		C.uintptr_t(len(bytes)),
	)
	defer C.webui_protocol_destroy(protocol)
	handler := C.webui_handler_create()
	defer C.webui_handler_destroy(handler)

	result, err := render(protocol, handler, `{"title":"Groceries"}`)
	if err != nil {
		fmt.Println(err)
		return
	}
	fmt.Println(result)
}
```

> **Memory note:** Converting the explicit-length slice to a Go string copies
> the bytes before the result is destroyed, including interior NUL bytes.
> Keep the FFI call and error retrieval on the same OS thread with
> `runtime.LockOSThread`.

## C\#

For .NET applications, prefer the managed `Microsoft.WebUI` NuGet package. It
wraps the same opaque native handles in `SafeHandle` types:

```csharp
using var protocol = new Protocol(File.ReadAllBytes("dist/protocol.bin"));
using var handler = new WebUIHandler("webui");
string html = handler.Render(
    protocol,
    """{"title":"Groceries"}""",
    "index.html",
    "/");
```

`Render` reads the full document through `webui_handler_render`: it copies
the borrowed bytes with `Marshal.PtrToStringUTF8(ptr, length)` and then calls
`webui_render_free`. This skips a terminator scan of large documents
and returns output that contains a `\0` character intact. Custom P/Invoke
bindings should mirror this lifecycle, receive returned pointers as `IntPtr`,
and copy other returned strings with `Marshal.PtrToStringUTF8` before releasing
them with `webui_free`.

The package also wraps the streaming session, so an ASP.NET endpoint can pace a
progressive response without touching the native ABI:

```csharp
using var session = handler.StreamResponse(protocol, "index.html", "/");
Response.ContentType = "text/html; charset=utf-8";
StreamingStep step = session.Start(initialState);
while (true)
{
    await Response.Body.WriteAsync(step.Bytes);
    await Response.Body.FlushAsync();
    if (step.Done) break;

    if (step.Boundary is BoundaryDescriptor boundary)
    {
        string state = await LoadStateAsync(
            boundary.Owner,
            boundary.Name,
            boundary.Key);
        step = session.Resume(boundary.InstanceId, state, BoundaryMode.Final);
    }
    else
    {
        step = session.Advance();
    }
}
```

Each call returns a `byte[]`, so `HttpResponse.Body` keeps its own write and
flush semantics. Failures throw `WebUIException` carrying the same diagnostic
`webui_last_error()` would report. See the [.NET integration](./dotnet).

## Other Languages

For a host with compatible C interop and a native WebUI build:

1. Load the shared library (`libwebui_ffi.dylib` / `.so` / `.dll`).
2. Declare the functions you need. For a server, use
   `webui_protocol_create`, `webui_handler_render`,
   `webui_render_bytes`, `webui_render_free`,
   `webui_protocol_destroy`, and `webui_last_error`.
3. Pass UTF-8 null-terminated strings for state, entry, and request path.
4. Check the return value, `NULL` means an error occurred.
5. Read the document bytes with their explicit length, copy or send them, then
   call `webui_render_free`.

## Next Steps

- [Plugins](/reference/plugins/), Plugin system and built-in plugin reference
- [CLI Reference](/reference/cli/), Building protocols with `webui build`
