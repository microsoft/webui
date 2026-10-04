// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, getTemplate, observable } from '../../../src/index.js';
import { cloneTemplateContent } from '../../../src/template-content.js';

class TestOwnerProbe extends WebUIElement {
  readonly constructedConnected = this.isConnected;
  readonly constructedOwnerDocument = this.ownerDocument;
  connectedCalls = 0;
  connectedOwnerDocument: Document | null = null;

  override connectedCallback(): void {
    super.connectedCallback();
    this.connectedCalls++;
    this.connectedOwnerDocument = this.ownerDocument;
  }
}

TestOwnerProbe.define('test-owner-probe');

class TestOwnerElement extends WebUIElement {
  @observable showConditional = true;
  @observable items = [{ id: 'first' }];
}

class TestOwnerShadow extends TestOwnerElement {}
TestOwnerShadow.define('test-owner-shadow');

class TestOwnerLight extends TestOwnerElement {}
TestOwnerLight.define('test-owner-light');

window.inspectTemplateCloneDocuments = () => {
  const probeMeta = getTemplate('test-owner-shadow');
  if (!probeMeta) throw new Error('owner-document fixture template is unavailable');
  const local = cloneTemplateContent(probeMeta, document);
  const iframe = document.createElement('iframe');
  document.body.appendChild(iframe);
  const target = iframe.contentDocument;
  if (!target) throw new Error('fixture iframe has no document');
  const foreign = cloneTemplateContent(probeMeta, target);
  const result = {
    localFragment: local.ownerDocument === document,
    localChildren: Array.from(local.childNodes).every(
      node => node.ownerDocument === document,
    ),
    foreignFragment: foreign.ownerDocument === target,
    foreignChildren: Array.from(foreign.childNodes).every(
      node => node.ownerDocument === target,
    ),
  };
  iframe.remove();
  return result;
};

declare global {
  interface Window {
    inspectTemplateCloneDocuments(): {
      localFragment: boolean;
      localChildren: boolean;
      foreignFragment: boolean;
      foreignChildren: boolean;
    };
  }
}
