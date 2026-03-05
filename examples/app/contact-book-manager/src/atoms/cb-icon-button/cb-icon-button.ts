import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbIconButton extends RenderableFASTElement(FASTElement) {
  @attr icon = '';
  @attr title = '';
  @attr variant = 'default';
}

CbIconButton.defineAsync({
  name: 'cb-icon-button',
  templateOptions: 'defer-and-hydrate',
});
