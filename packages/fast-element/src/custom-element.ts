type ConstructableHTMLElement = {
  new(...args: any[]): HTMLElement,
}

interface FASTElementDefinition {
  name: string
  template?: string
  templateInline?: string
  cssModule?: string
}

const cssModuleMap = new Map<string, string | CSSStyleSheet | HTMLElement>()

export function customElement(definition: FASTElementDefinition) {
  return function(type: ConstructableHTMLElement) {
    class NewConstructor extends type {
      internalCssModule: string | undefined
      constructor(...args: any[]) {
        super(...args)
        // Do stuff if ShadowDOM.
        if (this.children.length === 0) {
          if (!this.shadowRoot) {
            const shadowRoot = this.attachShadow({ mode: 'open' })
            this.internalStyleRender(shadowRoot)
            this.internalTemplateRender(shadowRoot)
          } else {
            this.internalStyleRender(this.shadowRoot)
          }
        }
      }

      internalTemplateRender(shadowRoot: ShadowRoot) {
        if (definition.templateInline) {
          const templateElement = document.getElementById(definition.templateInline) as HTMLTemplateElement | null
          if (templateElement) {
            const clone = templateElement.content.cloneNode(true)
            shadowRoot.appendChild(clone)
          }
        } else if (definition.template) {
          const template = document.createElement('template')
          template.innerHTML = definition.template
          shadowRoot.appendChild(template.content.cloneNode(true))
        }
      }

      internalStyleRender(shadowRoot: ShadowRoot) {
        if (definition.cssModule) {
          // Tag the cssModule to the element so we can read it while post build.
          this.internalCssModule = definition.cssModule

          // Not cached yet. Since we are using style tags for elminating flickr, caching the style tag so new
          // instances of the same component can reuse the same style tag.
          if (!cssModuleMap.has(definition.cssModule)) {
            // If we have a style tag, cache its content.
            const cssText = shadowRoot.querySelector('style')
            if (cssText?.textContent) {
              cssModuleMap.set(definition.cssModule, cssText)
            } else {
              const cssLink = shadowRoot.querySelector('link')
              if (!cssLink?.href) {
                const link = document.createElement('link')
                link.rel = 'stylesheet'
                link.href = definition.cssModule
                shadowRoot.prepend(link)
                cssModuleMap.set(definition.cssModule, link)
              } else {
                cssModuleMap.set(definition.cssModule, cssLink)
              }
              // Would be nice to use CSS modules but this fetches as "script" making it not cacheable.
              // import(definition.cssModule, { assert: { type: 'css' } })
              //   .then(module => {
              //     cssModuleMap.set(definition.cssModule!, module.default)
              //     shadowRoot.adoptedStyleSheets = [module.default]
              //   })
            }
          } else {
            // If we have a cached style tag, use it. Can be either a string or a CSSStyleSheet.
            const cssCachedItem = cssModuleMap.get(definition.cssModule)! as string | CSSStyleSheet | HTMLElement
            if (cssCachedItem instanceof CSSStyleSheet) {
              shadowRoot.adoptedStyleSheets = [cssCachedItem]
            } else if (cssCachedItem instanceof HTMLElement) {
              shadowRoot.prepend(cssCachedItem.cloneNode(true))
            } else {
              const css = document.createElement('style')
              css.textContent = cssCachedItem
              shadowRoot.prepend(css)
            }
          }
          // TODO: Reuse code if we can but we can't.
          // import(definition.cssModule, { assert: { type: 'css' }}).then((module) => {
          //   shadowRoot.adoptedStyleSheets = [module.default];
          // });
          // TODO: Cannot do link cause it isn't a module, it refetches.
          // const link = document.createElement('link');
          // link.rel = 'stylesheet';
          // link.href = definition.cssModule;
          // shadowRoot.prepend(link);
        }
      }
    }

    customElements.define(definition.name, NewConstructor)
  }
}
