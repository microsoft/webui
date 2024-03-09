# Build Time Rendering (BTR)

The Build Time Rendering Architecture, abbreviated as BTR, is a design that amalgamates the insights gained from integrating Islands, Server-Side Rendering (SSR), and Static Site Generation (SSG). Unlike traditional methods, BTR determines the rendering process in advance, guided by cues provided by the web application itself. A key feature of BTR is the introduction of a simplified streaming protocol. This protocol is language-agnostic, enabling web servers written in any programming language to deliver experiences akin to SSR, eliminating the dependency on Node.js.

# Enhanced Support for Service Workers with BTR

Service Workers play a pivotal role in the Build Time Rendering (BTR) architecture, primarily due to their ability to cache and manage resources. BTR generates a static rendering protocol file during the build process. This file, being static, can be cached on the client-side, significantly reducing the load on the server and improving the application's performance.

Service Workers can stream this protocol file, allowing for dynamic data to be fetched from the server and integrated into the client-side rendering process. This approach provides a balance between Server-Side Rendering (SSR) and Static Site Generation (SSG), leveraging the benefits of both while mitigating their respective drawbacks.

Compared to SSR, BTR is faster because it eliminates the need for a Node.js server to render every request. Instead, it relies on the Service Worker to serve cached static content, saving a significant amount of computational resources and bandwidth. This approach also reduces the server load, as the heavy lifting of rendering is offloaded to the client-side.

However, unlike SSG, BTR does require some server resources to fetch dynamic data as a JSON payload. This is a minor trade-off considering the enhanced user experience provided by dynamic content. SSG, while being the least resource-intensive, does not support dynamic data, which can limit the interactivity and real-time responsiveness of the application.

The integration of Service Workers with BTR provides a robust, efficient, and dynamic rendering solution. It combines the speed and efficiency of SSG, the dynamism and interactivity of SSR, and the resource optimization of client-side caching.

## Generation akin to Static Site Generation (SSG)

Incorporate hints within your web application pertaining to conditionals, lists, and variables. During the post-build process, these hints facilitate the division of the initially rendered page into streamable segments. The server no longer necessitates Node.js, as its primary function becomes reading the protocol of these streamable chunks and writing to the stream. Playwright is instrumental for the successful operation of BTR, as it traverses the DOM and flushes responses.

## Hydration resembling Islands Architecture

The first page rendered comprises solely of CSS and HTML, setting the stage for interactivity. The speed at which you can deliver the UI is contingent upon your web application.

Interactive elements are essentially Web Components. Upon page load, the HTML parser identifies unknown elements and maps them. Once the Web Components are registered, the parser is signaled to upgrade these to known elements. This process, known as Hydration, activates interactivity post-upgrade, establishing event listeners and bindings.

To seamlessly and instantaneously implement this without the user's awareness, we introduce a new concept of Declarative Bindings, including 'repeat', 'when', and 'signal'.

## Declarative Bindings

We introduce three declarative bindings to facilitate the BTR process in generating streamable chunks. These bindings adhere to the dot notation, thereby supporting any JSON/Object notations.

### f-signal

The `f-signal` binding serves as a wrapper around a value, capable of alerting relevant consumers when the value undergoes a change.

```html
<div f-signal=”name”>Default Name</div>
```

In this example, the `name` class variable within the Web Component is assigned the value "Default Name". Any mutation to `this.name` triggers an update in the corresponding DOM element.

### f-when

The `f-when` binding is a conditional element that can toggle between true and false states.

```html
<span f-when=”active”>Active</span>
```

In this instance, the `active` class variable within the Web Component is set to `true`. If the user alters this to `false`, the associated element ceases to be displayed.

### f-repeat

The `f-repeat` binding represents an array of templates.

```html
<div f-repeat=”users” w-component=”name-item”>
  <name-item>Red</name-item>
  <name-item>Blue</name-item>
  <name-item>Orange</name-item>
  <name-item>Yellow</name-item>
</div>
<template id=”name-item”>
  <slot></slot>
</template>
```

In this scenario, the `users` class variable within the Web Component is assigned a list of names, mirroring its current children. Similar to the other bindings, any mutation in this list prompts a corresponding mutation in the DOM.

## BTR Protocol and Server Handler

To facilitate server streaming and ensure compatibility with any language server, we require a protocol. The following definitions are provided for prototyping purposes.

```ts
interface BuildTimeRenderingStreamRepeat {
  type: 'repeat'
  value: string
  template: string
}

interface BuildTimeRenderingStreamRaw {
  type: 'raw'
  value: string
}

interface BuildTimeRenderingStreamSignal {
  type: 'signal'
  value: string
  defaultValue?: string
}

interface BuildTimeRenderingStreamWhen {
  type: 'when'
  value: string
}

type BuildTimeRenderingStream =
  | BuildTimeRenderingStreamRaw
  | BuildTimeRenderingStreamRepeat
  | BuildTimeRenderingStreamSignal
  | BuildTimeRenderingStreamWhen

interface BuildTimeRenderingTemplate {
  style: string
  template: string
}

type BuildTimeRenderingStreamTemplateRecords = Record<
  string,
  BuildTimeRenderingTemplate
>

interface BuildTimeRenderingProtocol {
  streams: BuildTimeRenderingStream[]
  templates: BuildTimeRenderingStreamTemplateRecords
}
```

The `BuildTimeRenderingProtocol` is an object generated by the BTR process. It functions as an append-only data structure, with each item possessing a unique method of writing to the response buffer. Currently, we support three response types: `Signal`, `Raw`, and `Repeat`. These types dictate the rendering approach for each stream partial.

- `Signal`: This type streams a dynamic variable to the stream buffer.
- `Raw`: This type streams static text to the stream buffer.
- `Repeat`: This type streams a template to the buffer N times. Each item can possess its own unique template.
- `When`: This type streams a styling attribute to hide the dom.

## Development

### How to Build and Run

```sh
# Builds the monorepo
pnpm build

# Runs the BTR process on the @example/shadowdom-app
pnpm btr

# Serves the website for @example/shadowdom-app
pnpm serve
```

### The components

#### The BTR Build in TypeScript

- **packages/fast-btr-build-js**: The BTR Build process, that converts a single page application into a build time rendered application. Produces the protocol json file.
- **packages/fast-btr-eval-js**: The BTR Expression Evaluator and Object query.
- **packages/fast-btr-parser-js**: The JavaScript Parser for the BTR protocol. Used when streaming a request. Useful for Node/Bun/JS/Service Workers.
- **packages/fast-btr-protocol-js**: The TypeScript bindings for the protocol response.

#### The BTR Parser in Rust

- **packages/fast-btr-parser-rust**: The Rust Parser for the BTR protocol. Used when streaming a request. Useful for super fast run web severs.

#### The BTR Hydration

- **packages/fast-element**: We have developed a prototype for a compact web components framework. This framework hydrates the declarative bindings (`f-when`, `f-signal`, `f-repeat`) to its `@observable` within the Web Component. The entire framework is lightweight, with a total size of 3.4KB, which compresses down to just 1.7KB when gzipped.
