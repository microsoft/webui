import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbStatCard extends RenderableFASTElement(FASTElement) {
  @attr icon = '';
  @attr value = '';
  @attr label = '';
}

CbStatCard.defineAsync({
  name: 'cb-stat-card',
  templateOptions: 'defer-and-hydrate',
});
