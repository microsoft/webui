import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbInput extends RenderableFASTElement(FASTElement) {
  @attr placeholder = '';
  @attr value = '';
  @attr type = 'text';
  @attr name = '';
}

CbInput.defineAsync({
  name: 'cb-input',
  templateOptions: 'defer-and-hydrate',
});
