---
layout: home
hero:
  name: "WebUI Framework"
  text: "Native Templates Everywhere"
  tagline: Write once, render natively across any platform without JavaScript
  image:
    src: /logo.svg
    alt: WebUI Logo
  actions:
    - theme: brand
      text: Get Started
      link: /guide/
    - theme: alt
      text: View on GitHub
      link: https://github.com/microsoft/webui

features:
  - title: True Cross-Platform
    details: Run the same templates in Rust, Go, .NET, or Node.js with zero runtime dependencies
  - title: Native Performance
    details: Built with Rust for unparalleled efficiency compared to JavaScript-based frameworks
  - title: Familiar Syntax
    details: Use standard HTML templates with intuitive directives - no new template language to learn
  - title: Minimal Footprint
    details: Small binary size with no bloated dependencies makes it ideal for all device types
  - title: Type Safe
    details: Strongly typed interfaces ensure reliable component integration across languages
  - title: Secure By Design
    details: No JavaScript eval() or runtime code execution means better security guarantees
---

# Welcome to WebUI Framework

WebUI is a revolutionary template engine that transforms standard HTML/CSS templates into a platform-agnostic protocol that can be rendered natively across any environment - with no JavaScript runtime dependencies.

```html
<!-- Simple HTML template -->
<h1>Hello, WebUI!</h1>
<if condition="hasItems">
  <ul>
    <for condition="item in items">
      <li>{{item.name}} - {{{item.description}}}</li>
    </for>
  </ul>
</if>
```

## Why WebUI?

Traditional web frameworks require a JavaScript runtime, leading to bloated dependencies and poor performance on resource-constrained devices. WebUI solves this by using a lightweight protocol that can be rendered natively in any language.

## Getting Started

Check out our [Quick Start Guide](/guide/quick-start) to build your first WebUI application in minutes!
