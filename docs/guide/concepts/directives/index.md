---
title: "Template directives"
description: "Choose bindings, conditions, loops, routes, streaming boundaries, and deferred hydration policies for HTML templates."
---

<a id="webui-directives"></a>

# Template Directives

Use directives to bind state and control which HTML renders. They are template
syntax, not JavaScript expressions or custom elements you register yourself.

```html
<if condition="items.length">
  <ul>
    <for each="item in items">
      <li>{{item.name}}</li>
    </for>
  </ul>
</if>
```

## Available Directives

Choose the directive for the task:

- [**`<if>` Conditional Rendering**](./if) - Conditionally render content based on expressions
- [**`<for>` Loop Iteration**](./for) - Iterate over collections to generate repeated content
- [**`<route>` Routing**](./route) - Map URL paths to components for SSR and optional client navigation
- [**`<boundary>` Streaming Boundary**](./boundary) - Discover and hydrate complete regions inside entries and reusable components
- [**Lazy Component Policy**](./lazy) - Defer offscreen rendering and hydration with root-template attributes
- [**`{{}}` Signal Binding**](./signals) - Insert dynamic values with automatic HTML escaping
- [**`{{{}}}` Raw Signal Binding**](./signals#raw-signals) - Insert unescaped HTML content
- [**Attribute Directives**](./attributes) - Bind dynamic data to HTML attributes (`{{}}`, `?`, `:`, and mixed)

## How Directives Work

The build validates directives and compiles them for the native renderer.
The renderer evaluates them against request state without executing application
JavaScript. In an interactive component, the browser framework also updates
bindings, conditions, and loops when state changes.

Text bindings read paths only. Conditions and boolean attributes support the
[documented expression subset](./if#supported-expression-patterns), not
arbitrary JavaScript.
