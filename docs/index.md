---
layout: home
hero:
  name: "WebUI Framework"
  text: "Fast Web Rendering Without JavaScript Overhead"
  tagline: Build UIs at compile time, serve them blazingly fast from any backend language
  actions:
    - theme: brand
      text: Get Started
      link: /guide/
    - theme: alt
      text: View on GitHub
      link: https://github.com/microsoft/webui
---

# Welcome to WebUI Framework

## A New Approach to Server-Side Rendering

WebUI Framework is a high-performance solution for building web applications that deliver exceptional user experiences without JavaScript runtime overhead. Unlike traditional frameworks that build UIs at runtime, WebUI separates dynamic from static content at build time and creates an efficient protocol for fast rendering.

### Why WebUI?

- **Language Agnostic Backend**: Use Rust, Go, C#, PHP, Ruby, or any other language - no Node.js required!
- **Optimized Web Vitals**: Significantly faster FCP, LCP, and INP metrics compared to JS-based SSR
- **Web Component-Based**: Built on the native web platform using modern web components
- **Edge-Ready**: Can be served from edge functions or a streamable Service Worker
- **Separation of Concerns**: UI structure is cached and separate from state data
- **Minimal Transfer Size**: Only send the state, not the entire rendered HTML

## How It Works

1. **Build Time**: WebUI discovers all your web components and creates a protocol that separates dynamic from static content
2. **Serving**: Your backend (in any language) applies state to this protocol through a lightweight handler
3. **Rendering**: Content reaches the browser pre-rendered and ready to display, with minimal JavaScript overhead

Get started now by following our [installation guide](/guide/) or check out the [Hello World tutorial](/tutorials/hello-world).
