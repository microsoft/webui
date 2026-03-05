import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbButton extends RenderableFASTElement(FASTElement) {
  @attr label = '';
  @attr variant = 'primary';
  @attr size = 'md';
}

CbButton.defineAsync({
  name: 'cb-button',
  templateOptions: 'defer-and-hydrate',
});
