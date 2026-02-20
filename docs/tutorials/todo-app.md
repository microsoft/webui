# Building a Todo App with WebUI Framework

This tutorial will guide you through building a simple Todo application using the WebUI Framework with Web Components. You'll learn how to:

1. Create a custom element
2. Use shadow DOM for encapsulation
3. Apply state and event handling
4. Implement add, complete, and delete functionality

## Project Setup

Create the following file structure:

```
todo-app/
├── components/
│   └── todo-item.css
│   └── todo-item.html
├── todo-app.css
└── index.html
```

## Creating the Todo Item

First, let's create the todo item style:

```css
:host {
    display: block;
}
.todo-item {
    display: flex;
    align-items: center;
    margin: 8px 0;
    padding: 8px;
    border-radius: 4px;
    background-color: var(--item-bg, #f9f9f9);
}
.todo-item.completed span {
    text-decoration: line-through;
    color: #888;
}
.todo-item button {
    margin-left: auto;
    background: #ff4d4d;
    color: white;
    border: none;
    border-radius: 4px;
    padding: 4px 8px;
    cursor: pointer;
}
```

Then create the todo item html:
```html
<div class="todo-item">
<input type="checkbox" id="checkbox">
<slot></span>
<button id="delete-btn">Delete</button>
</div>
```

## Creating the Main App

Now, let's create the main todo application styles:

```css
:host {
    display: block;
    max-width: 500px;
    margin: 0 auto;
    padding: 20px;
    font-family: 'Arial', sans-serif;
}
.add-todo {
    display: flex;
    margin-bottom: 20px;
}
.add-todo input {
    flex: 1;
    padding: 8px;
    border: 1px solid #ddd;
    border-radius: 4px 0 0 4px;
}
.add-todo button {
    background: #4CAF50;
    color: white;
    border: none;
    padding: 8px 16px;
    border-radius: 0 4px 4px 0;
    cursor: pointer;
}
.empty-state {
    text-align: center;
    color: #888;
    padding: 20px;
}
.todo-stats {
    margin-top: 20px;
    color: #666;
    font-size: 0.9em;
}
```

Then, let's create the main todo application html template:
```html
<h1>Todo App</h1>

<div class="add-todo">
    <input type="text" id="new-todo" placeholder="What needs to be done?">
    <button id="add-btn">Add</button>
</div>

<div id="todo-list" class="todo-list">
    <for each="todo in todos">
        <todo-item id="{{todo.id}}" completed="{{todo.completed}}">{{todo.text}}</todo-item>
    </for>
</div>

<div id="empty-state" class="empty-state" hidden>
    No todos yet. Add one above!
</div>

<div id="todo-stats" class="todo-stats"></div>
```

## Creating the Entry Point

```html
<!-- index.html -->
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>WebUI Todo App</title>
  <script type="module" src="todo-app.js"></script>
  <style>
    body {
      margin: 0;
      padding: 0;
      font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, 
                   Ubuntu, Cantarell, 'Open Sans', 'Helvetica Neue', sans-serif;
      background-color: #f5f5f5;
    }
  </style>
</head>
<body>
  <todo-app></todo-app>
</body>
</html>
```

## WebUI Framework Integration

To integrate this with WebUI Framework, your application state would be structured like:

```json
{
  "todos": [
    { "id": 1, "text": "Learn WebUI Framework", "completed": true },
    { "id": 2, "text": "Build a Todo App", "completed": false },
    { "id": 3, "text": "Deploy to production", "completed": false }
  ]
}
```

The WebUI Framework would handle rendering the initial state, and then the Web Component would take over for client-side interactivity.

## Backend Handler (Rust)

```rust
use webui_handler::{handler, WebUIProtocol};
use serde_json::Value;
use std::sync::{Arc, Mutex};

// In-memory todo store for this example
struct TodoStore {
    todos: Vec<Todo>,
    next_id: u32,
}

struct Todo {
    id: u32,
    text: String,
    completed: bool,
}

fn main() {
    // Initialize store with some data
    let store = Arc::new(Mutex::new(TodoStore {
        todos: vec![
            Todo { id: 1, text: "Learn WebUI Framework".to_string(), completed: true },
            Todo { id: 2, text: "Build a Todo App".to_string(), completed: false },
            Todo { id: 3, text: "Deploy to production".to_string(), completed: false },
        ],
        next_id: 4,
    }));
    
    // Set up your HTTP server here
    // For each request:
    
    // 1. Get the current state
    let current_state = get_state(&store);
    
    // 2. Load the protocol
    let protocol = load_protobuf("path/to/compiled/protocol.bin");
    
    // 3. Handle the request and write to response
    handler(protocol, current_state, |chunk| {
        // Write chunk to HTTP response
        response.write(chunk);
    }, || {
        // End the response
        response.end();
    });
}

// Helper function to convert our store to state JSON
fn get_state(store: &Arc<Mutex<TodoStore>>) -> Value {
    let store = store.lock().unwrap();
    
    // Convert to a format that matches our UI expectations
    json!({
        "todos": store.todos.iter().map(|t| {
            json!({
                "id": t.id,
                "text": t.text,
                "completed": t.completed
            })
        }).collect::<Vec<_>>()
    })
}
```

## Conclusion

You've now built a modern Todo application using WebUI Framework with proper Web Components. At runtime it will create them as declarative shadow dom.

This architecture combines the benefits of WebUI Framework's server-side rendering with client-side interactivity through standard Web Components, giving you the best of both worlds - fast initial load times and responsive user interactions.
