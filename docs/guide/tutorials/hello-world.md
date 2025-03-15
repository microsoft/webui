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
    <style>
        body {
            font-family: system-ui, sans-serif;
            max-width: 800px;
            margin: 0 auto;
            padding: 2rem;
        }
        .greeting {
            color: {{color}};
            font-size: 2rem;
            margin-bottom: 1rem;
        }
        .message {
            background: #f5f5f5;
            padding: 1rem;
            border-radius: 0.5rem;
        }
    </style>
</head>
<body>
    <div class="greeting">Hello, {{name}}!</div>
    
    <if condition="showMessage">
        <div class="message">{{{message}}}</div>
    </if>
    
    <h2>Your Information:</h2>
    <ul>
        <for condition="detail in details">
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

## Rendering with Rust

Let's create a Rust application to render this template:

```rust
use std::fs;
use serde_json::Value;
use webui_parser::{WebUIParser, Result};
use webui_handler::{handle, ResponseWriter};

struct FileWriter {
    file_path: String,
    content: String,
}

impl FileWriter {
    fn new(file_path: &str) -> Self {
        Self { 
            file_path: file_path.to_string(),
            content: String::new(),
        }
    }
    
    fn save(&self) -> std::io::Result<()> {
        fs::write(&self.file_path, &self.content)
    }
}

impl ResponseWriter for FileWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.content.push_str(content);
        Ok(())
    }
    
    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

fn main() -> Result<()> {
    // Parse the template
    let parser = WebUIParser::new();
    let protocol = parser.parse("src/templates/index.html", &["src/templates"])?;
    
    // Load the state
    let state_json = fs::read_to_string("state.json")?;
    let state: Value = serde_json::from_str(&state_json)?;
    
    // Render the template
    let mut writer = FileWriter::new("output.html");
    handle(&protocol, &state, &mut writer)?;
    writer.save()?;
    
    println!("Template rendered successfully to output.html");
    
    Ok(())
}
```

## Rendering with Go

Here's how you would render the same template with the Go handler:

```go
package main

import (
	"encoding/json"
	"fmt"
	"io/ioutil"
	"os"

	"github.com/mohamedmansour/webui/go/parser"
	"github.com/mohamedmansour/webui/go/handler"
	"github.com/mohamedmansour/webui/go/protocol"
)

func main() {
	// Parse the template
	p := parser.New()
	proto, err := p.Parse("src/templates/index.html", []string{"src/templates"})
	if err != nil {
		panic(err)
	}

	// Load the state
	stateFile, err := os.Open("state.json")
	if err != nil {
		panic(err)
	}
	defer stateFile.Close()

	var state map[string]interface{}
	if err := json.NewDecoder(stateFile).Decode(&state); err != nil {
		panic(err)
	}

	// Create output file
	outFile, err := os.Create("output.html")
	if err != nil {
		panic(err)
	}
	defer outFile.Close()

	// Create file writer
	writer := handler.NewFileWriter(outFile)
	
	// Render the template
	if err := handler.Handle(proto, state, writer); err != nil {
		panic(err)
	}

	fmt.Println("Template rendered successfully to output.html")
}
```

## Testing the Output

Open the generated `output.html` file in your browser to see the rendered output.

## What We've Learned

In this tutorial, we've:

1. Created a simple WebUI template with signals, conditionals, and loops
2. Defined a state object with the data for rendering
3. Used the WebUI parser to convert the template to the WebUI protocol
4. Rendered the protocol with the WebUI handler to generate HTML

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
