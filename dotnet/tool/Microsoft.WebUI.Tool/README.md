# Microsoft.WebUI.Tool

CLI tool for building and inspecting WebUI templates.

## Installation

```bash
dotnet tool install -g Microsoft.WebUI.Tool
```

## Usage

```bash
# Build templates into a binary protocol file
webui build ./src --output app.webui

# Inspect a compiled protocol file
webui inspect app.webui

# Start a dev server with hot reload
webui serve ./src --state ./data/state.json --port 3001 --watch
```

## Configuration

The tool locates the native `webui` binary using:

1. `WEBUI_BINARY_PATH` environment variable (directory containing the binary)
2. System PATH

## License

MIT
