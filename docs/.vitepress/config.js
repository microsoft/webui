export default {
  title: "WebUI Framework",
  description: "Native HTML Templating for Every Platform",
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
          { text: 'Components', link: '/guide/components/' },
          { text: 'Directives', link: '/guide/directives/' },
          { text: 'Handlers', link: '/guide/handlers/' },
          { text: 'CLI', link: '/guide/cli/' },
        ]
      },
      { text: 'Tutorials',
        items: [
          { text: 'Hello World', link: '/tutorials/hello-world.md' },
          { text: 'Todo App', link: '/tutorials/todo-app.md' },
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
            { text: 'Introduction', link: '/guide/' },
            { text: 'Installation', link: '/guide/installation' },
            { text: 'Quick Start', link: '/tutorials/hello-world' },
            { text: 'CLI Reference', link: '/guide/cli/', items: [
              { text: 'webui build', link: '/guide/cli/#webui-build' },
              { text: 'webui inspect', link: '/guide/cli/#webui-inspect' },
              { text: 'webui serve', link: '/guide/cli/#webui-serve' },
            ]},
          ]
        },
        {
          text: 'Core Concepts',
          items: [
            { text: 'State Management', link: '/guide/concepts/state-management' },
            { text: 'Components', link: '/guide/concepts/components' },
            { text: 'CSS Token Hoisting', link: '/guide/concepts/css-tokens' },
            {
              text: 'Template Directives',
              items: [
                { text: 'Overview', link: '/guide/concepts/directives/' },
                { text: '<if> Conditional', link: '/guide/concepts/directives/if' },
                { text: '<for> Loop', link: '/guide/concepts/directives/for' },
                { text: '<route> Routing', link: '/guide/concepts/directives/route' },
                { text: '{{}} Signals', link: '/guide/concepts/directives/signals' },
                { text: 'Attribute Directives', link: '/guide/concepts/directives/attributes' },
              ]
            },
            { text: 'Routing', link: '/guide/concepts/routing' },
            { text: 'Hydration & Interactivity', link: '/guide/concepts/hydration', items: [
              { text: 'Class Definition', link: '/guide/concepts/hydration#class-definition' },
              { text: 'Templating', link: '/guide/concepts/hydration#templating' },
              { text: 'Observation', link: '/guide/concepts/hydration#observation' },
              { text: 'Events', link: '/guide/concepts/hydration#events' },
              { text: 'References', link: '/guide/concepts/hydration#references' },
              { text: 'Initial State', link: '/guide/concepts/hydration#initial-state' },
            ]},
            {
              text: 'Platform Handlers',
              items: [
                { text: 'Overview', link: '/guide/concepts/handlers/' },
                { text: 'Rust', link: '/guide/concepts/handlers/rust' },
                { text: 'Node', link: '/guide/concepts/handlers/node' },
                { text: 'Electron', link: '/guide/concepts/handlers/electron' },
                { text: 'WebAssembly', link: '/guide/concepts/handlers/wasm' },
                { text: 'FFI (C API)', link: '/guide/concepts/handlers/ffi' }
              ]
            },
            { text: 'Plugins', link: '/guide/concepts/plugins/' }
          ]
        }
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
