import { FASTElement, customElement, observer } from '@internal/fast-element'
import template from './app-header.template.js'

@customElement({ name: 'app-header', template, cssModule: './app-header.css' })
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
