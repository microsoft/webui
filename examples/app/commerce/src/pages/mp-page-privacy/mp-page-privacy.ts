import { FASTElement } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MpPagePrivacy extends RenderableFASTElement(FASTElement) {}

MpPagePrivacy.defineAsync({
  name: 'mp-page-privacy',
  templateOptions: 'defer-and-hydrate',
});
