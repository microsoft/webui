import { FASTElement, customElement, observer } from '@btjs/element'

@customElement({ name: 'counter-element' })
export class Counter extends FASTElement {
  @observer
  counter = 0

  counterChanged() {
    this.updateServer()
  }

  onIncrementClick() {
    this.counter++
  }

  onDecrementClick() {
    this.counter--
  }

  onResetClick() {
    this.counter = 0
  }

  private updateServer() {
    fetch('/api/update', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ counter: this.counter }),
    })
  }
}
