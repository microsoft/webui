import { FASTElement } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MpPageShipping extends RenderableFASTElement(FASTElement) {}

MpPageShipping.defineAsync({
  name: 'mp-page-shipping',
  templateOptions: 'defer-and-hydrate',
});
