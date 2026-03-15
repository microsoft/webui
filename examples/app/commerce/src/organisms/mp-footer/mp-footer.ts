import { FASTElement } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MpFooter extends RenderableFASTElement(FASTElement) {}

MpFooter.defineAsync({
  name: 'mp-footer',
  templateOptions: 'defer-and-hydrate',
});
