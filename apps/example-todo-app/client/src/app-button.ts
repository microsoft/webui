import { FASTElement, customElement } from '@btjs/element'

@customElement({ name: 'app-button' })
export class AppButton extends FASTElement {
  static get observedAttributes() {
    return ['appearance']
  }

  attributeChangedCallback(name: string, _oldValue: string, newValue: string) {
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
