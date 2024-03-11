import { FASTElement, customElement, observer } from '@internal/fast-element'

@customElement({ name: 'counter-element' })
export class Counter extends FASTElement {
  @observer
  counter = 0

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
