import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CalcDisplay extends RenderableFASTElement(FASTElement) {
  @attr expression = '';
  @attr value = '';
  @attr error = '';
}

CalcDisplay.defineAsync({
  name: 'calc-display',
  templateOptions: 'defer-and-hydrate',
});
