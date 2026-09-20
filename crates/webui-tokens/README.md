# microsoft-webui-tokens

Shared CSS token helpers for RUST servers to use with CSS token hoisting.

## Overview

Design token loading, filtering, and CSS generation for the WebUI framework.
Token resolution emits declarations for parser token candidates present in each
theme, following present transitive `var(--token)` dependencies while trusting
theme internals.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT — Copyright (c) Microsoft Corporation.