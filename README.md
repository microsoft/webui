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

## How It Works

1. **Write Standard HTML/CSS**: Create templates using familiar HTML and CSS with WebUI-specific directives
2. **Parse to WebUI Protocol**: Templates compile to a lightweight, platform-agnostic protocol
3. **Native Rendering**: The platform-specific handler renders the protocol directly in your language of choice
4. **No JavaScript Required**: Everything runs natively in your application environment

## Project Structure

```
webui/
├── crates/                      # Rust crates
│   ├── webui-expressions/       # Expression evaluation engine
│   ├── webui-ffi/               # Expression evaluation engine
│   ├── webui-handler/           # Core protocol definitions
│   ├── webui-parser/            # Core protocol definitions
│   ├── webui-protocol/          # HTML/CSS/JS parser
│   └── webui-state/             # Main library (re-exports)
├── handlers/                    # Language-specific handlers
│   ├── node/                    # Node.js implementation
│   ├── go/                      # Go implementation
│   └── csharp/                  # C# implementation
├── examples/                    # Example applications
├── tests/                       # Integration tests
└── benchmarks/                  # Performance benchmarks
```

## Getting Started

### Prerequisites

- Rust toolchain (1.80+)
- Node.js (22+) with pnpm
- Go (1.18+) - optional
- .NET SDK (8.0+) - optional

### Building

```bash
# Clone the repository
git clone https://github.com/mohamedmansour/webui.git
cd webui

# Build Rust components
cargo build --release

# Build all handlers
pnpm install
pnpm build
```

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
