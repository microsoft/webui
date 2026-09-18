// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, registerTemplateData, WebUIElement } from '../../../src/index.js';

export class TestSuccessorPlain extends WebUIElement {
  @observable prefix = '';
  @observable between = '';
  @observable rawPrefix = '';
  @observable staticPrefix = '';
  @observable tail = '';
  @observable value = '';
  @observable show = false;
  @observable items: string[] = [];
  @observable html = '';
}
TestSuccessorPlain.define('test-successor-plain');

export class TestSuccessorCalls extends TestSuccessorPlain {}
TestSuccessorCalls.define('test-successor-calls');

// Ordinary authored comments are stripped by the source compiler. Raw metadata
// deliberately retains them to exercise the supported static-comment locator.
registerTemplateData({
  'test-successor-comments': {
    h: '<span><!----></span><!--root-->',
    tx: [[[1, 0], [['prefix']], 7], [[1, 1], [['suffix']]], [[0, 1], [['root']], 7]],
    tr: ['prefix', 'suffix', 'root'], ta: ['prefix', 'suffix', 'root'],
  },
  'test-successor-fragment-comments': {
    h: '', u: [[0, [0, 0]]],
    b: [{
      h: '<span><!----></span><!--root-->',
      tx: [[[1, 0], [['prefix']], 7], [[1, 1], [['suffix']]], [[0, 1], [['root']], 7]],
    }],
    tr: ['prefix', 'suffix', 'root'], ta: ['prefix', 'suffix', 'root'],
  },
});

export class TestSuccessorComments extends WebUIElement {
  @observable prefix = '';
  @observable suffix = '';
  @observable root = '';
}
TestSuccessorComments.define('test-successor-comments');
export class TestSuccessorFragmentComments extends TestSuccessorComments {}
TestSuccessorFragmentComments.define('test-successor-fragment-comments');
