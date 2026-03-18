# WebUI Directives

WebUI Framework offers several directives that allow you to create dynamic templates without JavaScript. These directives are processed during rendering to produce the final HTML output.

## Available Directives

WebUI Framework provides the following core directives:

- [**`<if>` Conditional Rendering**](./if) - Conditionally render content based on expressions
- [**`<for>` Loop Iteration**](./for) - Iterate over collections to generate repeated content
- [**`<route>` Routing**](./route) - Define client-side routes that map URL paths to components
- [**<code v-pre>{{}}</code> Signal Binding**](./signals) - Insert dynamic values with automatic HTML escaping
- [**<code v-pre>{{{}}}</code> Raw Signal Binding**](./signals#raw-signals) - Insert unescaped HTML content
- [**Attribute Directives**](./attributes) - Bind dynamic data to HTML attributes (<code v-pre>{{}}</code>, `?`, `:`, and mixed)

## How Directives Work

WebUI directives are processed by the WebUI parser and transformed into a platform-agnostic protocol. This protocol is then rendered by a language-specific handler, allowing the same template to be rendered consistently across multiple platforms.

```
Template with directives → WebUI Protocol → Native HTML output
```

The WebUI protocol is a protobuf binary structure defined by a cross-language .proto schema, making it ideal for cross-platform applications.
