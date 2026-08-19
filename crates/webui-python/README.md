# Microsoft WebUI for Python

`microsoft-webui` renders compiled WebUI protocols directly in Rust from
CPython. It ships prebuilt native wheels, so supported platforms do not need a
Rust toolchain or a separately installed WebUI shared library.

```bash
python -m pip install microsoft-webui
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
an iterable of tags. Streaming calls require state explicitly, including
`finish(final_state)`, so the document tail cannot accidentally render against
an empty state.

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
