import { FASTElement, customElement } from '@internal/fast-element'

const template = '<button><slot></slot></button>'

@customElement({ name: 'app-button', template, cssModule: '/button.css' })
export class AppButton extends FASTElement {
  static get observedAttributes() {
    return ['appearance']
  }

  attributeChangedCallback(name: string, oldValue: string, newValue: string) {
    if (name === 'appearance') {
      this.updateStyle(newValue)
    }
  }

  updateStyle(appearance: string) {
    const button = this.shadowRoot!.querySelector('button')
    if (button) {
      button.className = appearance
    }
  }

  connectedCallback() {
    super.connectedCallback()
    this.updateStyle(this.getAttribute('appearance') || '')
  }
}
