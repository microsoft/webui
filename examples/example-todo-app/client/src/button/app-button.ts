import { FASTElement, attr, customElement } from '@btjs/element'

@customElement({ name: 'app-button' })
export class AppButton extends FASTElement {
  @attr
  appearance: string | undefined

  appearanceChanged(_oldValue: string, newValue: string) {
    console.log(_oldValue, newValue)
  }
}
