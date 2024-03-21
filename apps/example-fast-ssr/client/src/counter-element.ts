import { FASTElement, customElement, html, observable } from '@microsoft/fast-element'

const template = html<Counter>`
  <span>${x => x.counter}</span>
  <button @click=${x => x.onDecrementClick()}>-</button>
  <button @click=${x => x.onIncrementClick}>+</button>
  <button @click=${x => x.onResetClick}>reset</button>
`

@customElement({ name: 'counter-element', template })
export class Counter extends FASTElement {
  @observable
  counter = 0

  counterChanged() {
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
}
