# Python

The `microsoft-webui` package provides high-performance server-side
rendering for Python. It is a direct **PyO3** native extension — not a
`ctypes` wrapper around the [C API](./ffi) — imported as `microsoft_webui`.
Rendering calls release the GIL, so a `Renderer` shared across threads keeps
rendering while the rest of your application runs Python code.

## Installation

> The `microsoft-webui` package is **not published to PyPI yet**. Wheels and a
> source distribution are built and attached to each GitHub Release; install one
> directly, or build from a checkout with `maturin`.

```bash
# From a GitHub Release asset
pip install ./microsoft_webui-<version>-cp311-abi3-<platform>.whl

# Or from a checkout
pip install maturin
maturin build --release --manifest-path crates/webui-python/Cargo.toml
```

The package ships prebuilt wheels for CPython 3.11+ on Windows, macOS, and
Linux (manylinux), each for x86_64 and ARM64, plus one source distribution.
See [Wheel matrix and scope](#wheel-matrix-and-scope) for what isn't covered
and why.

## Buffered rendering

Load `protocol.bin` once and reuse the `Renderer` for the process lifetime:

```python
from microsoft_webui import Renderer

renderer = Renderer.from_file("dist/protocol.bin", plugin="webui")

html = renderer.render(
    {"title": "Home"},
    entry="index.html",
    request_path="/",
)  # -> bytes
```

`render()` returns `bytes`, the canonical fast path for writing directly to a
socket or WSGI/ASGI response. Use `render_text()` when you need a `str`:

```python
html_str = renderer.render_text({"title": "Home"}, request_path="/")
```

### WSGI

```python
from wsgiref.simple_server import make_server
from microsoft_webui import Renderer

renderer = Renderer.from_file("dist/protocol.bin", plugin="webui")

def app(environ, start_response):
    html = renderer.render(
        {"title": "Home"},
        request_path=environ.get("PATH_INFO", "/"),
    )
    start_response("200 OK", [
        ("Content-Type", "text/html; charset=utf-8"),
        ("Content-Length", str(len(html))),
    ])
    return [html]

if __name__ == "__main__":
    with make_server("", 8000, app) as httpd:
        httpd.serve_forever()
```

### ASGI (Starlette / FastAPI)

`renderer.render()` is synchronous and CPU-bound. It releases the GIL inside
Rust, but it still blocks the calling Python thread, so an ASGI app should
offload it rather than call it straight from the event loop:

```python
import anyio  # or asyncio.to_thread on stdlib-only setups
from starlette.applications import Starlette
from starlette.responses import Response
from starlette.routing import Route
from microsoft_webui import Renderer

renderer = Renderer.from_file("dist/protocol.bin", plugin="webui")

async def index(request):
    html = await anyio.to_thread.run_sync(
        lambda: renderer.render({"title": "Home"}, request_path=request.url.path),
    )
    return Response(html, media_type="text/html; charset=utf-8")

app = Starlette(routes=[Route("/{path:path}", index)])
```

`Renderer` is thread-safe, so many worker threads can render through the same
instance concurrently — offloading to a thread pool doesn't introduce a race.

## Partial navigation and component templates

```python
partial = renderer.render_partial(
    {"title": "Home"},
    entry="index.html",
    request_path="/users/42",
    inventory=client_inventory_hex,
)  # -> bytes, complete JSON partial-navigation response

templates = renderer.render_component_templates(
    ["user-card", "avatar"],
    inventory=client_inventory_hex,
)  # -> bytes, requested component template payloads
```

Both return `bytes`; write them directly as the response body for
`Router.ensureLoaded()` / partial-navigation requests from the client.

## Progressive (host-driven) streaming

`renderer.stream_response()` opens a `StreamingSession` whose methods return
the bytes they produced instead of writing anywhere themselves. **WebUI never
touches your socket, so your server owns the write order and backpressure —
the same contract as every other host binding** (see
[Streaming Boundaries](/guide/concepts/directives/boundary#3-drive-a-host-controlled-response)
for the authoring side and full ordering rules).

### WSGI

WSGI's response is already an iterable of `bytes` chunks, so a generator maps
directly onto the session:

```python
def app(environ, start_response):
    session = renderer.stream_response(request_path=environ.get("PATH_INFO", "/"))
    rows = session.boundary("rows")  # resolved once, outside the write loop

    def body():
        yield session.write_shell({"title": "Home"})
        yield session.write_boundary(rows, {"rows": []}, mode="updatable")
        yield session.update(rows, {"rows": load_rows()})
        yield session.finish({})

    start_response("200 OK", [
        ("Content-Type", "text/html; charset=utf-8"),
        ("X-Accel-Buffering", "no"),
    ])
    return body()
```

### ASGI (Starlette / FastAPI)

Each session call is still synchronous and GIL-releasing, not `async`, so
offload it the same way as buffered rendering:

```python
import anyio
from starlette.responses import StreamingResponse

async def index(request):
    session = renderer.stream_response(request_path=request.url.path)
    rows = session.boundary("rows")

    async def body():
        yield await anyio.to_thread.run_sync(session.write_shell, {"title": "Home"})
        yield await anyio.to_thread.run_sync(
            lambda: session.write_boundary(rows, {"rows": []}, mode="updatable"),
        )
        rows_data = await load_rows()  # your own async backend call
        yield await anyio.to_thread.run_sync(session.update, rows, {"rows": rows_data})
        yield await anyio.to_thread.run_sync(session.finish, {})

    return StreamingResponse(body(), media_type="text/html; charset=utf-8")
```

Ordering is enforced by the session itself: the shell first, boundaries in
declaration order, `update()` only on boundaries already committed
`"updatable"`, and `finish()` last. A rejected call raises and leaves the
session usable, so bad state input doesn't cost you the response. Sessions
are **not** thread-safe — drive one session from one thread at a time;
independent sessions on the same `Renderer` may run concurrently.

## API reference

| Python | Description |
|--------|-------------|
| `Renderer(protocol_bytes, *, plugin=None)` | Decode and index protocol bytes once, binding an optional named plugin |
| `Renderer.from_file(path, *, plugin=None)` | Read `path` and construct a `Renderer` from its bytes |
| `renderer.render(state, *, entry="index.html", request_path="/")` | Render into `bytes` |
| `renderer.render_text(state, *, entry="index.html", request_path="/")` | Render and decode into `str` |
| `renderer.render_partial(state, *, entry="index.html", request_path="/", inventory="")` | Complete JSON partial-navigation response as `bytes` |
| `renderer.render_component_templates(tags, *, inventory="")` | On-demand component template payloads as `bytes` |
| `renderer.tokens` | `tuple[str, ...]` of CSS token names in build order |
| `renderer.stream_response(*, entry="index.html", request_path="/", nonce=None, head_inject=None, body_inject=None)` | Open a host-driven `StreamingSession` |

### StreamingSession

| Member | Description |
|--------|-------------|
| `boundary(name) -> int` | Resolve an authored boundary name to its integer handle. Raises with the valid names and a "did you mean …?" suggestion on a typo |
| `boundary_count` | Number of boundaries the entry declares |
| `finished` | Whether `finish()` successfully emitted the terminal record |
| `write_shell(state) -> bytes` | Document prefix through the first semantic flush |
| `write_boundary(id, state, mode=BoundaryMode.FINAL) -> bytes` | One boundary's markup, metadata delta, and checkpoint |
| `update(id, state) -> bytes` | Projected state patch for a boundary already committed `updatable` |
| `finish(state) -> bytes` | Tail checkpoint, terminal record, and document suffix |

### `Plugin` and `BoundaryMode`

Both are `enum.StrEnum` (standard in CPython 3.11+, this package's minimum
interpreter version), so they compare equal to plain strings while staying
typo-checked by static analysis:

```python
from microsoft_webui import BoundaryMode

session.write_boundary(rows, state, mode=BoundaryMode.UPDATABLE)
session.write_boundary(rows, state, mode="updatable")  # equivalent
```

## Fast path: pre-serialized state

Every `state` parameter accepts a Python `Mapping`, which the facade
serializes with the standard library `json` module before crossing into
Rust. If you already have JSON — cached, streamed from another service, or
assembled with a faster serializer — pass `str`, `bytes`, `bytearray`, or a
`memoryview` directly. Every pre-serialized form bypasses `json.dumps`;
immutable `str` and `bytes` remain backed by their Python objects while Rust
renders with the GIL released, while mutable/general buffers are copied first
for safety:

```python
import json

renderer.render({"title": "Home"})               # facade calls json.dumps for you
renderer.render(json.dumps({"title": "Home"}))    # str fast path
renderer.render(b'{"title":"Home"}')              # bytes fast path
```

## Head/body injection

`head_inject`, `body_inject`, and the reserved `$webui` state channel's
`headEnd` / `bodyStart` / `bodyEnd` members are written **verbatim** at their
structural boundary, exactly like every other host binding.

<webui-blockquote appearance="warning" title="Trusted HTML, not escaped input" icon="⚠️">

These fields are not escaped. Passing user-controlled content here is a
direct cross-site scripting (XSS) vector. Only pass fully trusted content
(build-time-derived markup, a dev livereload script) or content you have
already escaped yourself.

</webui-blockquote>

## Wheel matrix and scope

`microsoft-webui` builds against PyO3's `abi3-py311` stable ABI, so one wheel
per platform serves every CPython 3.11+ interpreter — no per-minor-version
build matrix. v1 ships five wheels: x86_64 and ARM64 for Windows and manylinux,
plus ARM64 for macOS, alongside one `sdist`.

v1 is **runtime-only**: it renders compiled protocols and does not expose a
build/compile API. Produce `protocol.bin` with `webui build` (the npm or Rust
CLI) — Python consumes the compiled artifact like every other host.

Not covered in v1: PyPy, GraalPy, free-threaded CPython builds, musllinux, and
32-bit architectures. The self-contained `sdist` bundles the matching WebUI
Rust source closure and lets a Rust toolchain build any of those targets, but
doing so is unsupported and untested. If you're on one of those platforms
today, the [C API's `ctypes` fallback](./ffi#advanced-fallback-ctypes) can still
reach WebUI directly.

## Next Steps

- [C / FFI](./ffi), the underlying C ABI and its `ctypes` fallback
- [Streaming Boundaries](/guide/concepts/directives/boundary), authoring side and ordering rules
- [Plugins](/guide/concepts/plugins/), plugin system and built-in plugin reference
