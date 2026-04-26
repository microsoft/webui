---
layout: home
hero:
  name: "WebUI Framework"
  text: "Server-render Web Components from any language — 4× faster than Node."
  tagline: The first SSR framework that doesn't need JavaScript on the server. Templates compile to a binary protocol; Rust, Go, Python, C#, or Node just stream bytes. The browser hydrates without a runtime.
  actions:
    - theme: brand
      text: Get Started →
      link: /guide/
    - theme: alt
      text: Why WebUI?
      link: /guide/why
    - theme: alt
      text: See benchmarks
      link: /guide/concepts/performance
    - theme: alt
      text: View on GitHub
      link: https://github.com/microsoft/webui

features:
  - icon: ⚡
    title: Compiled, not interpreted
    details: Templates and Web Components compile at build time into a binary Protocol Buffer. The server's hot path is just filling holes and streaming bytes — no template parsing, no AST walking, no interpretation at runtime.
  - icon: 🌐
    title: Render from any language
    details: SSR has historically been a Node-only privilege. WebUI breaks that ceiling. Stream pages from Rust, Go, Python, C#, Bun, Deno — or Node, if you must. The protocol is just bytes; any language that can read bytes and write strings can serve a site.
  - icon: 🪶
    title: Zero runtime, on either side
    details: No framework on the server. No framework on the page. Static content is plain HTML; interactive Web Components hydrate against the existing DOM using the native browser primitives. Your users download the page, not the framework.
  - icon: 🧩
    title: webui-framework
    details: A tiny client runtime for Web Components, tuned for rendering speed and low memory. Built on Declarative Shadow DOM and the native browser primitives — no virtual DOM, no proprietary component model, no JSX.
  - icon: 🧭
    title: webui-router
    details: A client-side router that hydrates incrementally, route by route, from server-delivered templates. Navigation reuses the same compiled protocol as the initial render — no double work, no waterfalls, no bundle splitting tax.
  - icon: 📰
    title: webui-press
    details: A blazing-fast static site generator powered by the framework itself. Compiles markdown and components in parallel into a fully static site — perfect for GitHub Pages, CDNs, and anywhere else that just serves files. (You're reading a site built with it.)
---
