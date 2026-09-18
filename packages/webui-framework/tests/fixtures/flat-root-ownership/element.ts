// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, registerTemplateData, WebUIElement } from '../../../src/index.js';
import { EMPTY_BINDINGS } from '../../../src/element/types.js';
import type { ScopeFrame, TemplateInstance } from '../../../src/element/types.js';
import type { TemplateBlockMeta, TemplateMeta, TemplateNodeIndex } from '../../../src/template-types.js';

// Exact top-level cardinalities and authored comments are intentional metadata
// fixtures, not a second template compiler or a measurement workload.
const leaf: TemplateBlockMeta = { h: '<span></span>', tx: [[[1, 0], [['label']]]] };
const plain: TemplateMeta = {
  ...leaf, sd: 1, a: [['title', 0, 'label']], ag: [[1, 0, 1]], re: [['click', 'clicked', []]],
};
const templates: Record<string, TemplateMeta> = {
  'test-flat-zero': { h: '', sd: 1, re: [['click', 'clicked', []]] },
  'test-flat-one': plain,
  'test-flat-two': { ...plain, h: '<span></span><!--authored-->' },
  'test-flat-many': { ...plain, h: '<span></span><!--authored-->literal<b>static</b><i></i><hr>' },
  'test-flat-empty-text': { h: '', sd: 1, tx: [[[0, 0], [['label']]]], c: [], r: [], u: [], re: [['click', 'clicked', []]] },
  'test-owned-condition': { h: '', sd: 1, c: [[[(resolve) => Boolean(resolve('show')), ['show']], 0, [0, 0]]], b: [leaf] },
  'test-owned-repeat': { h: '', sd: 1, r: [['items', 'item', 0, [0, 0]]], b: [{ h: '<span></span>', tx: [[[1, 0], [['item']]]] }] },
  'test-owned-raw': { h: '', sd: 1, tx: [[[0, 0], [['html']], 1]] },
  'test-owned-fragment': { h: '', sd: 1, u: [[0, [0, 0]]], b: [leaf] },
  'test-owned-host': {
    h: '', sd: 1, c: [[[(resolve) => Boolean(resolve('show')), ['show']], 0, [0, 0]]],
    b: [{ h: '<test-flat-one></test-flat-one>' }],
  },
};
registerTemplateData(templates);

/** Test-only observation; no instrumentation is added to framework instances. */
export class FlatRootProbe extends WebUIElement {
  @observable label = 'client';
  @observable show = true;
  @observable items = ['client'];
  @observable html = '<em>client raw</em>';
  fixtureRoot?: TemplateInstance;
  omittedDuringWiring = false;
  finalizations = 0;
  clicks = 0;
  hydrations = 0;

  clicked(): void { this.clicks++; }

  protected override hydratedCallback(): void { this.hydrations++; }

  protected override $finalize(
    instance: TemplateInstance,
    root: Node,
    meta: TemplateBlockMeta,
    resolver: (root: Node, index: TemplateNodeIndex) => Node | null,
    scope?: ScopeFrame,
    elements?: Array<Node | undefined>,
  ): void {
    if (!this.fixtureRoot) {
      this.fixtureRoot = instance;
      this.omittedDuringWiring = instance.nodes === EMPTY_BINDINGS;
    }
    this.finalizations++;
    super.$finalize(instance, root, meta, resolver, scope, elements);
  }

  override $destroy(): void {
    super.$destroy();
    this.fixtureRoot = undefined;
  }

  replaceRegistration(): void {
    registerTemplateData({ [this.localName]: { h: '<aside>replacement registration</aside>', sd: 1 } });
  }
}

export class TestFlatZero extends FlatRootProbe {}
TestFlatZero.define('test-flat-zero');
export class TestFlatOne extends FlatRootProbe {}
TestFlatOne.define('test-flat-one');
export class TestFlatTwo extends FlatRootProbe {}
TestFlatTwo.define('test-flat-two');
export class TestFlatMany extends FlatRootProbe {}
TestFlatMany.define('test-flat-many');
export class TestFlatEmptyText extends FlatRootProbe {}
TestFlatEmptyText.define('test-flat-empty-text');
export class TestOwnedCondition extends FlatRootProbe {}
TestOwnedCondition.define('test-owned-condition');
export class TestOwnedRepeat extends FlatRootProbe {}
TestOwnedRepeat.define('test-owned-repeat');
export class TestOwnedRaw extends FlatRootProbe {}
TestOwnedRaw.define('test-owned-raw');
export class TestOwnedFragment extends FlatRootProbe {}
TestOwnedFragment.define('test-owned-fragment');
export class TestOwnedHost extends FlatRootProbe {}
TestOwnedHost.define('test-owned-host');
