import { FASTElement, customElement, observer } from '@btjs/element'

interface Item {
  id: number
  name: string
}

@customElement({ name: 'app-shell' })
export class AppElement extends FASTElement {
  @observer
  toast: string | null = null

  @observer
  items: Item[] = []

  private toastId = 0
  private itemId = 0

  connectedCallback(): void {
    super.connectedCallback()

    this.addEventListener('item-removed', (event: Event) => {
      const customEvent = event as CustomEvent<string>
      this.setToast(`${customEvent.detail} removed`)
      this.setItems(this.items.filter((item) => item.name !== customEvent.detail))
    })

    this.addEventListener('item-renamed', (event: any) => {
      const index = Array.from(event.detail.target.parentNode.children).indexOf(event.detail.target)
      this.setToast(`Renamed to ${event.detail.textContent}`)
      this.setItems([...this.items.slice(0, index), event.detail.textContent, ...this.items.slice(index + 1)])
    })
  }

  onClick(_e: Event) {
    this.setItems([...this.items, {
      id: this.itemId,
      name: `Item ${++this.itemId}`,
    }])
    this.setToast(`Item ${this.itemId} added`)
  }

  onClear(_e: Event) {
    this.setItems([])
    this.setToast('Items cleared')
  }

  setItems(items: Item[]) {
    this.items = items
    fetch('/api/items', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(items),
    })
      .catch((error) => {
        console.error('Error:', error)
      })
  }

  setToast(msg: string | null) {
    this.toast = msg
    if (msg) {
      clearTimeout(this.toastId)
      this.toastId = window.setTimeout(() => {
        this.toast = null
      }, 2000)
    }
  }
}
