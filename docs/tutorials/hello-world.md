# Hello World Tutorial

In this tutorial, we'll build a simple "Hello World" application using WebUI, demonstrating the basics of the framework.

## Project Setup

First, let's create a new project directory structure:

```
hello-world/
├── src/
│   └── templates/
│       └── index.html
└── state.json
```

## Creating the Template

Create a simple template in `src/templates/index.html`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{{title}}</title>
    <link rel="stylesheet" href="/assets/styles.css">
</head>
<body>
    <div class="greeting">Hello, {{name}}!</div>
    
    <if condition="showMessage">
        <div class="message">{{{message}}}</div>
    </if>
    
    <h2>Your Information:</h2>
    <ul>
        <for each="detail in details">
            <li><strong>{{detail.label}}:</strong> {{detail.value}}</li>
        </for>
    </ul>
</body>
</html>
```

## Creating the State

Create a `state.json` file with the data for rendering:

```json
{
    "title": "WebUI Hello World",
    "name": "WebUI Developer",
    "color": "#0066cc",
    "showMessage": true,
    "message": "<em>Welcome to <strong>WebUI</strong>!</em>",
    "details": [
        {"label": "Framework", "value": "WebUI"},
        {"label": "Version", "value": "1.0.0"},
        {"label": "Status", "value": "Learning"}
    ]
}
```

## Previewing with `webui start`

The fastest way to preview your app during development is the built-in dev server:

```bash
webui-cli start ./hello-world/templates --state ./hello-world/data/state.json --servedir ./hello-world/assets --watch
```

This will:
1. Build the protocol from your templates
2. Render the HTML with your state data
3. Serve the result at `http://127.0.0.1:3000/`
4. With `--watch`, watch for file changes and automatically reload

Open `http://127.0.0.1:3000/` in your browser to see the rendered output. With `--watch`, editing `templates/index.html` or `data/state.json` triggers automatic reload.

You can also specify a custom port:

```bash
webui-cli start ./hello-world/templates --state ./hello-world/data/state.json --servedir ./hello-world/assets --watch --port 9090
```

## Building for Production

To produce a `protocol.bin` for use with a handler in production:

```bash
webui build ./hello-world/templates --out ./hello-world/dist
```

## Rendering with Rust

Here's how to render a pre-built protocol programmatically with Rust:

```rust
use std::fs;
use serde_json::Value;
use webui_protocol::WebUIProtocol;
use webui_handler::{WebUIHandler, ResponseWriter};

struct StdoutWriter;

impl ResponseWriter for StdoutWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        print!("{content}");
        Ok(())
    }
    
    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let protocol = WebUIProtocol::from_protobuf_file("dist/protocol.bin")?;
    let state: Value = serde_json::from_str(&fs::read_to_string("data/state.json")?)?;
    
    let mut handler = WebUIHandler::new();
    let mut writer = StdoutWriter;
    handler.handle(&protocol, &state, &mut writer)?;
    
    Ok(())
}
```

## What We've Learned

In this tutorial, we've:

1. Created a WebUI app folder with templates, data, and assets
2. Used `webui start` to preview the app with live reload
3. Built the protocol with `webui build` for production use
4. Rendered the protocol programmatically with the Rust handler

This demonstrates the core workflow of WebUI:

```
HTML Template → WebUI Protocol → Rendered HTML
```

## Next Steps

Now that you've built a simple Hello World application, try:

- Adding more complex conditions and loops
- Creating custom components
- Using nested state data
- Trying different handler implementations (Node.js, .NET)
