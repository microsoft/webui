import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbAvatar extends RenderableFASTElement(FASTElement) {
  @attr initials = '';
  @attr color = '#6B7280';
  @attr size = 'md';
}

CbAvatar.defineAsync({
  name: 'cb-avatar',
  templateOptions: 'defer-and-hydrate',
});
