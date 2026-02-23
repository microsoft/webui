export default {
  title: "WebUI Framework",
  description: "Native HTML Templating for Every Platform",
  head: [
    ['link', { rel: 'icon', href: '/favicon.ico' }],
  ],
  themeConfig: {
    logo: '/logo.svg',
    siteTitle: 'WebUI Framework',
    
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
            { text: 'CLI Reference', link: '/guide/cli/' },
          ]
        },
        {
          text: 'Core Concepts',
          items: [
            { text: 'State Management', link: '/guide/concepts/state-management' },
            { text: 'Components', link: '/guide/concepts/components' },
            {
              text: 'Template Directives',
              items: [
                { text: 'Overview', link: '/guide/concepts/directives/' },
                { text: '<if> Conditional', link: '/guide/concepts/directives/if' },
                { text: '<for> Loop', link: '/guide/concepts/directives/for' },
                { text: '{{}} Signals', link: '/guide/concepts/directives/signals' },
                { text: 'Attribute Directives', link: '/guide/concepts/directives/attributes' },
              ]
            },
            {
              text: 'Platform Handlers',
              items: [
                { text: 'Overview', link: '/guide/concepts/handlers/' },
                { text: 'Rust', link: '/guide/concepts/handlers/rust' }
              ]
            }
          ]
        },
        {
          text: 'Advanced Topics',
          items: [
            { text: 'WebUI Protocol', link: '/guide/advanced/protocol' },
            { text: 'Performance Optimization', link: '/guide/advanced/performance' },
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
      '/playground/': [
        {
          text: 'Playground',
          items: [
            { text: 'Interactive Demo', link: '/playground/' },
            { text: 'Examples', link: '/playground/examples' },
          ]
        }
      ],
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
