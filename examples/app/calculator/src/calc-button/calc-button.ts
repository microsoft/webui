import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CalcButton extends RenderableFASTElement(FASTElement) {
  @attr label = '';
  @attr value = '';
  @attr({ attribute: 'btn-type' }) btnType = '';
  @attr({ attribute: 'btn-span' }) btnSpan = '';

  onClick(): void {
    this.dispatchEvent(
      new CustomEvent('button-press', {
        bubbles: true,
        composed: true,
        detail: { value: this.value },
      })
    );
  }
}

CalcButton.defineAsync({
  name: 'calc-button',
  templateOptions: 'defer-and-hydrate',
});
