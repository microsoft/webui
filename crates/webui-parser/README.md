# microsoft-webui-parser

HTML/CSS template parser for the [WebUI](https://github.com/microsoft/webui) framework. Transforms WebUI template markup into the binary protocol consumed by the handler at runtime.

## Overview

`microsoft-webui-parser` uses tree-sitter to parse HTML and CSS, extracting static and dynamic fragments, component slots, directives (`<for>`, `<if>`), and CSS token bindings into a compact protobuf protocol.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT — Copyright (c) Microsoft Corporation.
