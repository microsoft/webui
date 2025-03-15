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
      { text: 'Guide', link: '/guide/tutorials/hello-world.md' },
      { text: 'Directives', link: '/directives/' },
      { text: 'Handlers', link: '/handlers/' },
      { text: 'API', link: '/api/' },
    ],
    
    // Sidebar navigation
    sidebar: {
      '/guide/': [
        {
          text: 'Getting Started',
          items: [
            { text: 'Introduction', link: '/guide/' },
            { text: 'Installation', link: '/guide/installation' },
            { text: 'Quick Start', link: '/guide/quick-start' },
          ]
        },
        {
          text: 'Core Concepts',
          items: [
            { text: 'WebUI Protocol', link: '/guide/protocol' },
            { text: 'State Management', link: '/guide/state-management' },
            { text: 'Components', link: '/guide/components' },
          ]
        },
        {
          text: 'Tutorials',
          items: [
            { text: 'Hello World', link: '/guide/tutorials/hello-world' },
            { text: 'Todo App', link: '/guide/tutorials/todo-app' },
          ]
        }
      ],
      '/directives/': [
        {
          text: 'Template Directives',
          items: [
            { text: 'Overview', link: '/directives/' },
            { text: '<if> Conditional', link: '/directives/if' },
            { text: '<for> Loop', link: '/directives/for' },
            { text: '{{}} Signals', link: '/directives/signals' },
          ]
        }
      ],
      '/handlers/': [
        {
          text: 'Platform Handlers',
          items: [
            { text: 'Overview', link: '/handlers/' },
            { text: 'Rust', link: '/handlers/rust' },
            { text: 'Go', link: '/handlers/go' },
            { text: 'Node.js', link: '/handlers/node' },
            { text: '.NET', link: '/handlers/dotnet' },
          ]
        }
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
    },
    
    // Social links
    socialLinks: [
      { icon: 'github', link: 'https://github.com/mohamedmansour/webui' }
    ],
    
    // Footer
    footer: {
      message: 'Released under the MIT License',
      copyright: 'Copyright © 2023-present WebUI Contributors'
    },
    
    // Search
    search: {
      provider: 'local'
    }
  }
}
