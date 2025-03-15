# WebUI Handlers

WebUI handlers are the bridge between the WebUI protocol and the final rendered HTML output. They process the protocol and render the content in a specific programming language.

## Available Handlers

WebUI provides official handlers for several popular programming languages:

- [**Rust**](./rust) - High-performance native rendering with the Rust programming language
- [**Go**](./go) - Fast and efficient rendering with the Go programming language
- [**Node.js**](./node) - JavaScript-based rendering for Node.js applications
- [**.NET**](./dotnet) - C# implementation for .NET applications

## How Handlers Work

All WebUI handlers follow the same pattern:

1. They accept a WebUI protocol object (usually parsed from JSON)
2. They process the protocol with the provided state data
3. They render the final HTML output by evaluating directives and inserting dynamic content

This consistent approach ensures that the same template produces identical results across different programming languages and platforms.

## Common Handler Interface

While the specific implementation details vary between languages, all handlers provide a similar API:

```
handle(protocol, state, writer)
```

Where:
- `protocol` is the WebUI protocol object
- `state` is the data object with values to be rendered
- `writer` is a callback or interface for writing the rendered output

## Handler Customization

Each handler implementation can be extended or customized to fit specific requirements. See the individual handler documentation for language-specific details.
