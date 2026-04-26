# WebUI Press

WebUI Framework Powered Static Site Generator

[![microsoft-webui-press on crates.io](https://img.shields.io/badge/crate-microsoft--webui--press-orange)](https://github.com/microsoft/webui)

## Overview

`microsoft-webui-press` transforms a directory of markdown files into a full static site. It parses frontmatter, renders markdown to HTML with syntax highlighting (via syntect), pre-expands Declarative Shadow DOM for Web Components in content, and generates a search index — all in parallel using rayon. The site itself is rendered server-side using the [WebUI](https://github.com/microsoft/webui) framework, eating its own dog food.

## Features

- Markdown → HTML with syntect syntax highlighting
- Parallel page rendering via rayon
- DSD pre-expansion for Web Components in content
- Search index generation
- Frontmatter + sidebar-driven content pipeline
- Custom CSS theme support via design tokens
- Custom pages with state files for component-driven content

## Usage

```bash
webui-press build
```

By default, looks for `.webui-press/config.json`.

### Configuration (config.json)

```json
{
  "site": {
    "title": "My Site",
    "description": "Project documentation"
  },
  "basePath": "/",
  "contentDir": ".",
  "outDir": "./dist",
  "css": "./theme.css",
  "nav": [
    { "text": "Guide", "link": "/guide/" }
  ],
  "sidebar": [
    {
      "title": "Getting Started",
      "items": [
        { "text": "Introduction", "link": "/guide/" }
      ]
    }
  ]
}
```

## Documentation

Full documentation: https://microsoft.github.io/webui

## License

MIT — Copyright (c) Microsoft Corporation.
