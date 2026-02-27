# WebUI

A high-performance, truly cross-platform web framework leveraging native web technologies, built with Rust.

## Overview

WebUI is a framework that transforms standard HTML/CSS templates into a platform-agnostic protocol that can be rendered natively across any environment - without JavaScript runtime dependencies. Dynamic Server Side Rendering without Node.js.

The framework represents a paradigm shift in web development:

- **Truly Cross-Platform**: Run identical templates in Rust, Go, .NET, and other environments without requiring Node.js or any JavaScript runtime
- **Native Performance**: Built with Rust for unparalleled efficiency and minimal overhead compared to JavaScript-based frameworks
- **Familiar Developer Experience**: Uses standard HTML templates, web components, and CSS you already know
- **Zero Runtime Dependencies**: No need for Node.js, npm modules, or browser polyfills - templates run directly on native code
- **Type Safety**: Strongly typed interfaces ensure reliable component integration across language boundaries
- **Small Footprint**: Minimal binary size with no bloated dependencies

WebUI enables you to write web-based UIs once and deploy them anywhere - from cloud servers to embedded devices - with consistent rendering and superior performance.

## Core Architecture
WebUI follows a modular architecture with four primary components:

- **Protocol:** Defines the structural representation of UI components using a serializable format
- **Parser:** Processes HTML/CSS templates into protocol structures at build time
- **Expression Evaluation:** Handles conditional logic for dynamic rendering
- **Handler:** Renders protocol with state data into final HTML output at runtime

## How It Works
- **Write Standard HTML/CSS:**  Create templates with familiar syntax plus WebUI directives (`<for>`, `<if>`, `{{signals}}`)
- **Parse to WebUIProtocol:**  Templates compile to a lightweight, language-agnostic protocol at build time
- **Native Rendering:** The protocol is rendered with your data using a platform-specific handler in your language of choice
- **Efficient Output:** The handler produces optimized HTML with Web Component support

## Getting Started

### Prerequisites

- Rust toolchain (1.80+)
- Node.js (22+) with pnpm
- Go (1.18+) - optional
- .NET SDK (8.0+) - optional

### Building

```bash
# Clone the repository
git clone https://github.com/microsoft/webui.git
cd webui

# Build Rust components
cargo build --release
```

### Using the CLI

The `webui` CLI builds your app folder into the WebUI protocol format:

```bash
# Build an app (outputs protocol.bin and component CSS to the out folder)
cargo run -p webui-cli -- build ./my-app --out ./dist

# Specify a custom entry file
cargo run -p webui-cli -- build ./my-app --out ./dist --entry page.html

# Build the hello-world example
cargo run -p webui-cli -- build examples/hello-world --out ./dist
```

After building with `--release`, use the binary directly:

```bash
webui build ./my-app --out ./dist
```

### Building the WASM Playground

The interactive playground runs WebUI in the browser via WebAssembly. The WASM output is committed to the repo — most developers don't need to rebuild it. Only rebuild when you change Rust code in the core crates:

```bash
cargo xtask build-wasm
```

### Development Server

Preview your app using the built-in dev server. Add `--watch` to enable live reload:

```bash
webui-cli start examples/app/hello-world/templates --state examples/app/hello-world/data/state.json --servedir examples/app/hello-world/assets --watch
```

This builds, renders, and serves the app at `http://127.0.0.1:3000/`. With `--watch`, file changes trigger automatic reload.

For runnable sample apps and integration walkthroughs, see [examples/README.md](examples/README.md).

## Contributing

This project welcomes contributions and suggestions.  Most contributions require you to agree to a
Contributor License Agreement (CLA) declaring that you have the right to, and actually do, grant us
the rights to use your contribution. For details, visit https://cla.opensource.microsoft.com.

When you submit a pull request, a CLA bot will automatically determine whether you need to provide
a CLA and decorate the PR appropriately (e.g., status check, comment). Simply follow the instructions
provided by the bot. You will only need to do this once across all repos using our CLA.

This project has adopted the [Microsoft Open Source Code of Conduct](https://opensource.microsoft.com/codeofconduct/).
For more information see the [Code of Conduct FAQ](https://opensource.microsoft.com/codeofconduct/faq/) or
contact [opencode@microsoft.com](mailto:opencode@microsoft.com) with any additional questions or comments.

## Trademarks

This project may contain trademarks or logos for projects, products, or services. Authorized use of Microsoft 
trademarks or logos is subject to and must follow 
[Microsoft's Trademark & Brand Guidelines](https://www.microsoft.com/en-us/legal/intellectualproperty/trademarks/usage/general).
Use of Microsoft trademarks or logos in modified versions of this project must not cause confusion or imply Microsoft sponsorship.
Any use of third-party trademarks or logos are subject to those third-party's policies.

## License

MIT
