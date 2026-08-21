# Microsoft WebUI for Python

`microsoft-webui` renders compiled WebUI protocols directly in Rust from
CPython. It ships prebuilt native wheels, so supported platforms do not need a
Rust toolchain or a separately installed WebUI shared library.

> The `microsoft-webui` package is **not published to PyPI yet**. Wheels and a
> source distribution are built and attached to each GitHub Release; install one
> directly, or build from a checkout with `maturin`.

```bash
python -m pip install ./microsoft_webui-<version>-cp311-abi3-<platform>.whl
```

```python
from microsoft_webui import Plugin, Renderer

renderer = Renderer.from_file("dist/protocol.bin", plugin=Plugin.WEBUI)
html = renderer.render({"title": "Home"})
```

`render()` returns UTF-8 `bytes` for direct use by Python HTTP frameworks.
Pass pre-serialized JSON as `str` or `bytes` to bypass Python-side
serialization. Use `render_text()` only when an application specifically needs
a Python string.

`render_component_templates()` accepts either one component tag as a string or
an iterable of tags.

Host-driven streaming discovers runtime occurrences instead of resolving
compile-time names:

```python
from microsoft_webui import BoundaryMode

session = renderer.stream_response()
step = session.start({"title": "Home", "items": [{"id": 7}]})
while not step.done:
    boundary = step.boundary
    send(step.bytes)
    if boundary is not None:
        step = session.resume(
            boundary.instance_id,
            {"title": "Home"},
            mode=BoundaryMode.FINAL,
        )
    else:
        step = session.advance()
send(step.bytes)
```

`start()`, `resume()`, and `advance()` return immutable `StreamStep` values
containing the bytes produced by that call, a `done` flag, and an optional
`BoundaryDescriptor`. A descriptor means call `resume()`; no descriptor with
`done == False` means call `advance()`; `done == True` means complete.
`resume()` returns only the pending occurrence through its checkpoint, while
`advance()` returns following parent or tail bytes. No sibling boundary
workaround is required.

Optional boundary keys are Python strings, integers, or floats. They are
required only when one component-owned declaration is reached from multiple
static callsites in one entry traversal. `update(instance_id, patch)` returns
the update record as `bytes` and is valid between `resume()` and `advance()`.

The first release targets regular CPython 3.11+ builds on Windows, macOS, and
manylinux, on x64 and ARM64. PyPy, free-threaded CPython, and Alpine/musllinux
are not included initially.

Template compilation remains a build-time operation:

```bash
npx webui build ./src --out ./dist --plugin=webui
```

Building from source requires Rust 1.93 or newer. The source distribution
bundles the matching WebUI Rust source closure, so it does not require a
separately installed native library or separately published WebUI crates.

See the [Python integration
guide](https://microsoft.github.io/webui/guide/integrations/python) for partial
navigation and host-driven streaming examples.
