# Language Integrations

WebUI is designed to be language-agnostic. While the core engine is written in Rust, the FFI layer exposes a C API that any language can call. This means you can render WebUI templates from Go, C#, Python, or any language that supports C interop.

## Building the Shared Library

Build the shared library before using it from any language:

```bash
cargo build -p webui-ffi            # debug
cargo build -p webui-ffi --release  # release
```

This produces:

| Platform | Library file |
|---|---|
| macOS | `target/debug/libwebui_ffi.dylib` |
| Linux | `target/debug/libwebui_ffi.so` |
| Windows | `target/debug/webui_ffi.dll` |

## C API Reference

The library exports six functions. The generated C header is at `crates/webui-ffi/include/webui_ffi.h`.

### webui_render

```c
char *webui_render(const char *html, const char *data_json);
```

Parse an HTML template and render it with JSON state data in a single call. This is the **recommended entry point** for most consumers.

- `html` — null-terminated UTF-8 string containing the HTML template.
- `data_json` — null-terminated UTF-8 JSON string with the render state.
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

- The returned pointer is **owned by the library** — do **not** free it.
- The pointer is valid until the next FFI call on the same thread.
- Each thread has its own independent error state.

### webui_handler_create

```c
void *webui_handler_create();
```

Create a reusable handler instance. Returns an opaque pointer that must eventually be freed with `webui_handler_destroy`. Use this with `webui_handler_render` when rendering pre-compiled protobuf protocols.

### webui_handler_destroy

```c
void webui_handler_destroy(void *handler_ptr);
```

Destroy a handler instance created by `webui_handler_create`. Passing `NULL` is a safe no-op.

### webui_handler_render

```c
char *webui_handler_render(void *handler_ptr,
                           const uint8_t *protocol_data,
                           uintptr_t protocol_len,
                           const char *data_json);
```

Render a pre-compiled WebUI protocol (protobuf binary) with JSON state data. This is the lower-level API for callers that have already compiled their templates to protobuf via the CLI.

- `handler_ptr` — pointer returned by `webui_handler_create`.
- `protocol_data` — pointer to protobuf binary data.
- `protocol_len` — length of the protobuf data in bytes.
- `data_json` — null-terminated UTF-8 JSON string with the render state.
- **Returns** a heap-allocated string on success, or `NULL` on error.
- The caller **must** free the returned string with `webui_free()`.

## Memory Management

Two rules to remember:

1. **Free what you receive.** Every non-`NULL` string returned by `webui_render` or `webui_handler_render` is heap-allocated. You must free it with `webui_free()`.
2. **Don't free error strings.** The pointer from `webui_last_error()` is owned by the library. It remains valid until your next FFI call on the same thread.

| Pointer source | Who frees it? | How? |
|---|---|---|
| `webui_render` | Caller | `webui_free(ptr)` |
| `webui_handler_render` | Caller | `webui_free(ptr)` |
| `webui_last_error` | Library (do **not** free) | Automatically replaced on next call |
| `webui_handler_create` | Caller | `webui_handler_destroy(ptr)` |

## Python

Python's built-in `ctypes` module can load the shared library directly — no pip packages needed.

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
2. Declare the functions you need — at minimum `webui_render`, `webui_free`, and `webui_last_error`.
3. Pass UTF-8 null-terminated strings for `html` and `data_json`.
4. Check the return value — `NULL` means an error occurred.
5. Copy the returned string into your language's managed memory, then call `webui_free`.
