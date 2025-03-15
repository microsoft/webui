import { FASTElement, customElement, observer } from '@btjs/element'

@customElement({ name: 'app-header' })
export class AppHeaderElement extends FASTElement {
  @observer
  public appTitle: string | undefined

  headerElement: HTMLHeadingElement | undefined

  onChangeTitle() {
    this.appTitle = 'Demo time!'
    if (this.headerElement) {
      this.headerElement.style.color = 'red'
    }
  }
}
