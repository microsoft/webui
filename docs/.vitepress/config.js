// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export default {
  title: "WebUI Framework",
  description: "Language-Agnostic SSR with Web Components Islands Architecture",
  appearance: true,
  head: [
    ['link', { rel: 'icon', href: '/favicon.ico' }],
  ],
  themeConfig: {
    logo: '/logo.svg',
    siteTitle: 'WebUI Framework',
    aside: false,
    outline: false,
    
    // Navigation bar
    nav: [
      { text: 'Home', link: '/' },
      { 
        text: 'Guide',
        link: '/guide/',
        items: [
          { text: 'Why WebUI?', link: '/guide/why' },
          { text: 'How It Works', link: '/guide/concepts/how-it-works' },
          { text: 'Interactivity', link: '/guide/concepts/interactivity' },
          { text: 'Components', link: '/guide/concepts/components/' },
          { text: 'CLI Reference', link: '/guide/cli/' },
          { text: 'Integrations', link: '/guide/integrations' },
        ]
      },
      { text: 'Tutorials',
        items: [
          { text: 'Hello World', link: '/tutorials/hello-world' },
          { text: 'Todo App', link: '/tutorials/todo-app' },
        ]
       },
      { text: 'Playground', link: '/playground/' },
    ],
    
    // Sidebar navigation
    sidebar: {
      '/guide/': [
        {
          text: 'Getting Started',
          items: [
            { text: 'What is WebUI?', link: '/guide/' },
            { text: 'Why WebUI?', link: '/guide/why' },
            { text: 'Installation', link: '/guide/installation' },
            { text: 'Quick Start', link: '/tutorials/hello-world' },
          ]
        },
        {
          text: 'Core Concepts',
          items: [
            { text: 'How It Works', link: '/guide/concepts/how-it-works' },
            { text: 'Components', link: '/guide/concepts/components' },
            {
              text: 'Template Syntax',
              items: [
                { text: 'Overview', link: '/guide/concepts/directives/' },
                { text: 'Signals {{}}', link: '/guide/concepts/directives/signals' },
                { text: 'Conditionals <if>', link: '/guide/concepts/directives/if' },
                { text: 'Loops <for>', link: '/guide/concepts/directives/for' },
                { text: 'Attributes', link: '/guide/concepts/directives/attributes' },
                { text: 'Routes <route>', link: '/guide/concepts/directives/route' },
              ]
            },
            { text: 'Interactivity', link: '/guide/concepts/interactivity' },
            { text: 'State Management', link: '/guide/concepts/state-management' },
            { text: 'Routing', link: '/guide/concepts/routing' },
            { text: 'Hydration', link: '/guide/concepts/hydration' },
            { text: 'Performance', link: '/guide/concepts/performance' },
          ]
        },
        {
          text: 'Guides',
          items: [
            { text: 'CLI Reference', link: '/guide/cli/', items: [
              { text: 'webui build', link: '/guide/cli/#webui-build' },
              { text: 'webui serve', link: '/guide/cli/#webui-serve' },
              { text: 'webui inspect', link: '/guide/cli/#webui-inspect' },
            ]},
            { text: 'Language Integrations', link: '/guide/integrations' },
            { text: 'Best Practices', link: '/guide/concepts/best-practices' },
            { text: 'CSS Token Hoisting', link: '/guide/concepts/css-tokens' },
            { text: 'Plugins', link: '/guide/concepts/plugins/' },
            {
              text: 'Platform Handlers',
              items: [
                { text: 'Overview', link: '/guide/concepts/handlers/' },
                { text: 'Rust', link: '/guide/concepts/handlers/rust' },
                { text: 'Node / Bun / Deno', link: '/guide/concepts/handlers/node' },
                { text: 'Electron', link: '/guide/concepts/handlers/electron' },
                { text: 'WebAssembly', link: '/guide/concepts/handlers/wasm' },
                { text: 'FFI (C API)', link: '/guide/concepts/handlers/ffi' },
              ]
            },
          ]
        },
        {
          text: 'AI',
          items: [
            { text: 'AI Reference', link: '/guide/ai' },
          ]
        },
      ],
      '/tutorials': [
        {
          text: 'Examples',
          items: [
            { text: 'Hello World', link: '/tutorials/hello-world' },
            { text: 'Todo App', link: '/tutorials/todo-app' },
          ]
        }
      ],
      '/framework/': [
        {
          text: 'Components',
          items: [
            { text: 'Overview', link: '/framework/components/' },
          ]
        },
      
      ],
      '/api/': [
        {
          text: 'API Reference',
          items: [
            { text: 'Overview', link: '/api/' },
            { text: 'Protocol', link: '/api/protocol' },
            { text: 'Parser', link: '/api/parser' },
            { text: 'Handler', link: '/api/handler' },
            { text: 'Expressions', link: '/api/expressions' },
          ]
        }
      ],
      '/playground/': [],
    },
    
    // Social links
    socialLinks: [
      { icon: 'github', link: 'https://github.com/microsoft/webui' }
    ],
    
    // Footer
    footer: {
      message: 'Released under the MIT License',
      copyright: 'Copyright © 2025-present Microsoft'
    },
    
    // Search
    search: {
      provider: 'local'
    }
  }
}
