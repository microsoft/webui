# Microsoft.WebUI.Tool

CLI tool for building and inspecting WebUI templates.

## Installation

```bash
dotnet tool install -g Microsoft.WebUI.Tool
```

NuGet artifacts for this tool include this README, repository metadata, Source Link, a package license URL with license acceptance required, release notes links, discoverability tags, the `© Microsoft Corporation. All rights reserved.` notice, and `.snupkg` symbols. Release workflows stage the artifacts for downstream signing and publishing. NuGet.org publishing is not automatic until an approved Microsoft-certificate signing path is available for `.nupkg` packages. Before publishing, staged packages and Authenticode-signable contents must be signed with a Microsoft certificate through the approved signing process.

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
