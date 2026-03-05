import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbBadge extends RenderableFASTElement(FASTElement) {
  @attr label = '';
  @attr variant = 'default';
}

CbBadge.defineAsync({
  name: 'cb-badge',
  templateOptions: 'defer-and-hydrate',
});
