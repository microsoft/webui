import { FASTElement, css, customElement, html, observable } from '@microsoft/fast-element'

const template = html<Counter>`
  <span>${x => x.counter}</span>
  <button @click=${x => x.onDecrementClick()}>-</button>
  <button @click=${x => x.onIncrementClick()}>+</button>
  <button @click=${x => x.onResetClick()}>reset</button>
`

const styles = css`
:host {
  background-color: #ddd;
  margin: 10px;
  padding: 10px;
  display: inline-flex;
  align-items: center;
  gap: 10px;
  border-radius: 10px;
  user-select: none;
}

button {
  color: black;
  padding: 10px;
  border: none;
  cursor: pointer;
  border-radius: 10px;
  &:hover {
    outline: 1px solid #bbb;
    background-color: #ccc;
  }
}

span {
  font-size: 20px;
  font-family: 'Courier New', Courier, monospace;
  min-width: 50px;
  text-align: center;
}
`

@customElement({ name: 'counter-element', template, styles })
export class Counter extends FASTElement {
  @observable
  counter = 0

  counterChanged(prev, curr) {
    if (prev !== undefined) {
      this.updateServer()
    }
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
