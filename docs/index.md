---
layout: home
hero:
  name: "WebUI"
  text: "Web Rendering Without a JavaScript Runtime"
  tagline: Compile templates to binary. Serve them instantly from any backend.
  actions:
    - theme: brand
      text: Get Started →
      link: /guide/
    - theme: alt
      text: View on GitHub
      link: https://github.com/microsoft/webui

features:
  - icon: ⚡
    title: Compiled to Binary
    details: Templates are compiled at build time into a Protocol Buffer binary — no parsing or interpretation at runtime.
  - icon: 🌐
    title: Language Agnostic
    details: Render from Rust, Node, Bun, Deno, C#, Python, Go — or any language via FFI.
  - icon: 🔩
    title: Minimal Runtime Work
    details: Static and dynamic split at build time. At runtime the server fills in state and evaluates expressions — no client-side scripting for template logic.
  - icon: 🧩
    title: Web Components
    details: Built on native web components with Shadow DOM encapsulation for style isolation and reusability.
  - icon: 🔌
    title: Plugin System
    details: Parser and handler plugins for hydration, adding reactivity to interactive islands, custom directives, and framework-specific behavior.
  - icon: 🏗️
    title: Replaces Node.js SSR
    details: No JavaScript runtime on the server. Rust-native rendering uses less memory and handles more requests per second — fewer servers, lower bills.
---
