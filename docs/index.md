---
layout: home
hero:
  name: "WebUI Framework"
  text: "Compiled Templates. Any Backend. Interactive Web Components."
  tagline: Compile HTML templates to binary at build time. Render from Rust, Node, Go, C#, or Python at 4.3× the speed of Fastify. Hydrate only what's interactive.
  actions:
    - theme: brand
      text: Get Started →
      link: /guide/
    - theme: alt
      text: Why WebUI?
      link: /guide/why
    - theme: alt
      text: View on GitHub
      link: https://github.com/microsoft/webui

features:
  - icon: 🏝️
    title: Islands Architecture
    details: Each Web Component is an interactive island. Static content stays server-rendered with zero client-side JavaScript. Only components that need interactivity ship code to the browser.
  - icon: ⚡
    title: Compiled to Binary Protocol
    details: Templates are compiled at build time into Protocol Buffer binaries - no parsing, no AST walking, no interpretation at runtime. Static and dynamic content are separated once and never re-analyzed.
  - icon: 🌐
    title: Language Agnostic
    details: Render from Rust, Node, Bun, Deno, C#, Python, Go - or any language via FFI. The compiled protocol is just bytes; any backend that can read bytes and write strings can serve pages.
  - icon: 🧩
    title: Web Components
    details: Built on native Web Components with Declarative Shadow DOM. No virtual DOM, no proprietary component model - just the web platform with style encapsulation built in. Light DOM optionally available.
  - icon: 📐
    title: Declarative Templates
    details: HTML for structure, CSS for styling, TypeScript for behavior - in separate files. No JSX, no template literals, no CSS-in-JS. WebUI keeps concerns separated for performance and clarity.
  - icon: 🔌
    title: Plugin System
    details: Extensible parser and handler plugins for hydration strategies, custom directives, and framework-specific behavior. Control exactly how and when each island becomes interactive.
---
