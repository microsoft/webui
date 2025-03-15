import { FASTElement, customElement, observer } from '@btjs/element'

@customElement({ name: 'app-item' })
export class AppItemElement extends FASTElement {
  @observer
  name: string | undefined

  onRemove(_e: Event) {
    this.dispatchEvent(
      new CustomEvent('item-removed', {
        detail: this.name,
        bubbles: true,
        composed: true,
      }),
    )
  }

  onRename(_e: Event) {
    this.contentEditable = 'true'
    this.addEventListener('keydown', this.onEditableKeydown)
  }

  onEditableKeydown = (e: KeyboardEvent) => {
    if (e.key === 'Enter') {
      this.contentEditable = 'false'
      this.removeEventListener('keydown', this.onEditableKeydown)
      this.dispatchEvent(
        new CustomEvent('item-renamed', {
          detail: {
            textContent: this.textContent?.trim(),
            target: this,
          },
          bubbles: true,
          composed: true,
        }),
      )
    } else if (e.key === 'Escape') {
      this.contentEditable = 'false'
      this.removeEventListener('keydown', this.onEditableKeydown)
    }
  }
}
