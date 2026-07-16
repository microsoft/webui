// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Shared conformance fixtures for bundler-neutral projection adapters.
 */

import * as path from "node:path";
import type {
  AdapterContext,
  ModuleGraph,
  ModuleNode,
  OutputMembership,
  ResolvedImport,
} from "../graph.js";
import type {
  ComponentEntry,
  ProjectionManifest,
} from "../manifest.js";
import {
  VIRTUAL_HASH,
  compareUtf8,
  computeBuildId,
  hashContent,
  serializeManifestCanonical,
  validateManifestSchema,
} from "../manifest.js";

/** A single projection conformance scenario. */
export interface ConformanceCase {
  readonly id: string;
  readonly description: string;
  readonly scope: "compiler" | "rust";
  readonly graph: ModuleGraph;
  readonly membership: OutputMembership;
  readonly outputContents: ReadonlyMap<string, string | Uint8Array>;
  readonly expectedComponents: Readonly<Record<string, ComponentEntry>> | null;
  readonly expectedDiagnosticCodes: ReadonlyArray<string>;
}

/** Summary of one conformance run. */
export interface ConformanceReport {
  readonly passed: ReadonlyArray<string>;
  readonly failed: ReadonlyArray<ConformanceFailure>;
  readonly skipped: ReadonlyArray<string>;
}

/** One failed conformance assertion. */
export interface ConformanceFailure {
  readonly id: string;
  readonly reason: string;
  readonly expected: unknown;
  readonly actual: unknown;
}

/** Adapter/compiler entry point accepted by the suite. */
export type AdapterFactory = (
  context: AdapterContext
) => Promise<ProjectionManifest>;

/** Optional conformance selection. */
export interface ConformanceSuiteOptions {
  filter?: (value: ConformanceCase) => boolean;
}

const FIXTURE_ROOT = path.resolve(".webui-projection-conformance");
const MANIFEST_PATH = path.join(
  FIXTURE_ROOT,
  "dist",
  "webui-projection.json"
);

/**
 * Run all compiler-scoped fixtures, including byte-for-byte determinism and
 * full manifest structural/hash validation.
 */
export async function runConformanceSuite(
  adapterFactory: AdapterFactory,
  options: ConformanceSuiteOptions = {}
): Promise<ConformanceReport> {
  const selected = options.filter
    ? ALL_CASES.filter(options.filter)
    : ALL_CASES;
  const passed: string[] = [];
  const failed: ConformanceFailure[] = [];
  const skipped: string[] = [];

  for (const fixture of selected) {
    if (fixture.scope !== "compiler") {
      skipped.push(fixture.id);
      continue;
    }
    const context: AdapterContext = {
      graph: fixture.graph,
      membership: fixture.membership,
      outputContents: fixture.outputContents,
      rootDir: FIXTURE_ROOT,
      manifestPath: MANIFEST_PATH,
      bundlerName: "test",
      bundlerVersion: "0.0.0",
    };

    if (fixture.expectedComponents === null) {
      const actualCodes = await captureDiagnosticCodes(
        adapterFactory,
        context
      );
      if (!sameStrings(actualCodes, fixture.expectedDiagnosticCodes)) {
        failed.push({
          id: fixture.id,
          reason: "diagnostic codes differ",
          expected: fixture.expectedDiagnosticCodes,
          actual: actualCodes,
        });
      } else {
        passed.push(fixture.id);
      }
      continue;
    }

    try {
      const first = await adapterFactory(context);
      const second = await adapterFactory(context);
      const firstJson = serializeManifestCanonical(first);
      const secondJson = serializeManifestCanonical(second);
      if (firstJson !== secondJson) {
        failed.push({
          id: fixture.id,
          reason: "identical inputs produced different manifest bytes",
          expected: firstJson,
          actual: secondJson,
        });
        continue;
      }
      const mismatch = validateSuccessManifest(fixture, first);
      if (mismatch) {
        failed.push({
          id: fixture.id,
          reason: mismatch,
          expected: fixture.expectedComponents,
          actual: first,
        });
      } else {
        passed.push(fixture.id);
      }
    } catch (error: unknown) {
      failed.push({
        id: fixture.id,
        reason: "expected success but compilation failed",
        expected: fixture.expectedComponents,
        actual: error,
      });
    }
  }

  return { passed, failed, skipped };
}

async function captureDiagnosticCodes(
  factory: AdapterFactory,
  context: AdapterContext
): Promise<string[]> {
  try {
    await factory(context);
    return ["<no-error>"];
  } catch (error: unknown) {
    if (
      error instanceof Error &&
      "diagnostics" in error &&
      Array.isArray((error as { diagnostics: unknown }).diagnostics)
    ) {
      return (
        error as { diagnostics: Array<{ readonly code: string }> }
      ).diagnostics
        .map((diagnostic) => diagnostic.code)
        .sort(compareUtf8);
    }
    return ["<non-projection-error>"];
  }
}

function validateSuccessManifest(
  fixture: ConformanceCase,
  manifest: ProjectionManifest
): string | undefined {
  const schemaErrors = validateManifestSchema(manifest);
  if (schemaErrors.length > 0) {
    return `schema validation failed: ${schemaErrors.join(", ")}`;
  }
  if (manifest.root !== "..") {
    return `expected root "..", got "${manifest.root}"`;
  }
  if (manifest.adapter.name !== "test" || manifest.adapter.bundler !== "test@0.0.0") {
    return "adapter identity differs";
  }
  if (
    serializeComponents(manifest.components) !==
    serializeComponents(fixture.expectedComponents ?? {})
  ) {
    return "component entries differ";
  }

  const expectedInputs = expectedInputHashes(fixture.graph);
  if (
    JSON.stringify(manifest.inputs) !==
    JSON.stringify(expectedInputs)
  ) {
    return "input hashes differ";
  }
  const expectedOutputs = expectedOutputHashes(
    fixture.membership,
    fixture.outputContents
  );
  if (
    JSON.stringify(manifest.outputs) !==
    JSON.stringify(expectedOutputs)
  ) {
    return "output hashes differ";
  }

  const expectedBuildId = computeBuildId({
    producerName: manifest.producer.name,
    producerVersion: manifest.producer.version,
    adapterName: manifest.adapter.name,
    adapterBundler: manifest.adapter.bundler,
    root: manifest.root,
    analysisHash: manifest.analysisHash,
    sortedInputs: Object.entries(manifest.inputs),
    sortedOutputs: Object.entries(manifest.outputs),
    sortedComponents: Object.entries(manifest.components).map(
      ([tag, entry]) =>
        [
          tag,
          entry.module,
          entry.outputs,
          entry.hydrationKeys,
          entry.navigationKeys,
        ] as const
    ),
  });
  return manifest.buildId === expectedBuildId
    ? undefined
    : "buildId does not cover the canonical manifest proof";
}

function expectedInputHashes(
  graph: ModuleGraph
): Record<string, string> {
  const entries: Array<[string, string]> = [];
  for (const node of graph.modules.values()) {
    entries.push([
      canonicalId(node.id),
      node.kind === "virtual"
        ? VIRTUAL_HASH
        : hashContent(node.source ?? ""),
    ]);
  }
  entries.sort((left, right) => compareUtf8(left[0], right[0]));
  return Object.fromEntries(entries);
}

function expectedOutputHashes(
  membership: OutputMembership,
  contents: ReadonlyMap<string, string | Uint8Array>
): Record<string, string> {
  const entries: Array<[string, string]> = [];
  for (const outputId of membership.outputs.keys()) {
    entries.push([
      canonicalId(outputId),
      isVirtual(outputId)
        ? VIRTUAL_HASH
        : hashContent(contents.get(outputId)!),
    ]);
  }
  entries.sort((left, right) => compareUtf8(left[0], right[0]));
  return Object.fromEntries(entries);
}

function serializeComponents(
  value: Readonly<Record<string, ComponentEntry>>
): string {
  const entries = Object.entries(value).sort((left, right) =>
    compareUtf8(left[0], right[0])
  );
  return JSON.stringify(entries);
}

function sameStrings(
  left: ReadonlyArray<string>,
  right: ReadonlyArray<string>
): boolean {
  return (
    JSON.stringify([...left].sort(compareUtf8)) ===
    JSON.stringify([...right].sort(compareUtf8))
  );
}

function canonicalId(id: string): string {
  if (isVirtual(id)) {
    const bytes = Buffer.from(id.slice(1), "utf8");
    let result = "virtual:";
    for (const byte of bytes) {
      result += byte.toString(16).padStart(2, "0");
    }
    return result;
  }
  return path.relative(FIXTURE_ROOT, id).split(path.sep).join("/");
}

function isVirtual(id: string): boolean {
  return id.charCodeAt(0) === 0;
}

function fileId(relative: string): string {
  return path.join(FIXTURE_ROOT, relative);
}

function fileModule(
  relative: string,
  source: string,
  imports: ReadonlyArray<ResolvedImport> = []
): readonly [string, ModuleNode] {
  const id = fileId(relative);
  return [id, { id, kind: "file", source, imports }];
}

function virtualModule(
  id: string,
  source: string | undefined,
  imports: ReadonlyArray<ResolvedImport> = []
): readonly [string, ModuleNode] {
  return [id, { id, kind: "virtual", source, imports }];
}

function resolved(
  specifier: string,
  relative: string,
  kind: "static" | "dynamic" = "static",
  packageName?: string
): ResolvedImport {
  return {
    specifier,
    resolvedId: fileId(relative),
    external: false,
    kind,
    ...(packageName === undefined ? {} : { packageName }),
  };
}

function framework(
  specifier = "@microsoft/webui-framework"
): ResolvedImport {
  return {
    specifier,
    resolvedId: undefined,
    external: true,
    kind: "static",
    packageName: "@microsoft/webui-framework",
  };
}

function graph(
  modules: ReadonlyArray<readonly [string, ModuleNode]>,
  entries: ReadonlyArray<string>
): ModuleGraph {
  return { modules: new Map(modules), entries };
}

function emitted(
  outputs: ReadonlyArray<
    readonly [relative: string, members: ReadonlyArray<string>, content: string]
  >
): {
  readonly membership: OutputMembership;
  readonly outputContents: ReadonlyMap<string, string>;
} {
  const membership = new Map<string, ReadonlySet<string>>();
  const outputContents = new Map<string, string>();
  for (const [relative, members, content] of outputs) {
    const id = fileId(relative);
    membership.set(id, new Set(members));
    outputContents.set(id, content);
  }
  return { membership: { outputs: membership }, outputContents };
}

function component(
  module: string,
  outputs: ReadonlyArray<string>,
  hydrationKeys: ReadonlyArray<string>,
  navigationKeys: ReadonlyArray<string> = hydrationKeys
): ComponentEntry {
  return {
    module,
    outputs,
    hydrationKeys,
    navigationKeys,
  };
}

function success(
  id: string,
  description: string,
  fixtureGraph: ModuleGraph,
  output: ReturnType<typeof emitted>,
  expectedComponents: Readonly<Record<string, ComponentEntry>>
): ConformanceCase {
  return {
    id,
    description,
    scope: "compiler",
    graph: fixtureGraph,
    membership: output.membership,
    outputContents: output.outputContents,
    expectedComponents,
    expectedDiagnosticCodes: [],
  };
}

function failure(
  id: string,
  description: string,
  fixtureGraph: ModuleGraph,
  output: ReturnType<typeof emitted>,
  codes: ReadonlyArray<string>
): ConformanceCase {
  return {
    id,
    description,
    scope: "compiler",
    graph: fixtureGraph,
    membership: output.membership,
    outputContents: output.outputContents,
    expectedComponents: null,
    expectedDiagnosticCodes: codes,
  };
}

function rustCase(id: string, description: string): ConformanceCase {
  return {
    id,
    description,
    scope: "rust",
    graph: { modules: new Map(), entries: [] },
    membership: { outputs: new Map() },
    outputContents: new Map(),
    expectedComponents: {},
    expectedDiagnosticCodes: [],
  };
}

const contactId = fileId("src/contact-card.ts");
const contactGraph = graph(
  [
    fileModule(
      "src/contact-card.ts",
      `
import { observable, attr, WebUIElement } from '@microsoft/webui-framework';
class ContactCard extends WebUIElement {
  @observable email = '';
  @attr firstName = '';
  @attr({ attribute: 'last-name' }) lastName = '';
}
ContactCard.define('contact-card');
`,
      [framework()]
    ),
    fileModule("src/index.ts", "import './contact-card.ts';", [
      resolved("./contact-card.ts", "src/contact-card.ts"),
    ]),
  ],
  [fileId("src/index.ts")]
);

/** Canonical compiler and host conformance cases. */
export const ALL_CASES: ReadonlyArray<ConformanceCase> = [
  success(
    "basic-single-entry",
    "Direct decorators use exact JavaScript property names.",
    contactGraph,
    emitted([
      [
        "dist/index.js",
        [fileId("src/index.ts"), contactId],
        "bundle:contact",
      ],
    ]),
    {
      "contact-card": component(
        "src/contact-card.ts",
        ["dist/index.js"],
        ["email", "firstName", "lastName"]
      ),
    }
  ),
  success(
    "empty-keys",
    "A proven WebUI class with no reactive properties emits empty surfaces.",
    graph(
      [
        fileModule(
          "src/static-banner.ts",
          `
import { WebUIElement } from '@microsoft/webui-framework';
class StaticBanner extends WebUIElement {}
StaticBanner.define('static-banner');
`,
          [framework()]
        ),
      ],
      [fileId("src/static-banner.ts")]
    ),
    emitted([
      [
        "dist/index.js",
        [fileId("src/static-banner.ts")],
        "bundle:static",
      ],
    ]),
    {
      "static-banner": component(
        "src/static-banner.ts",
        ["dist/index.js"],
        []
      ),
    }
  ),
  success(
    "aliased-decorator",
    "Named decorator aliases preserve framework identity.",
    graph(
      [
        fileModule(
          "src/counter.ts",
          `
import { observable as obs, WebUIElement } from '@microsoft/webui-framework';
class Counter extends WebUIElement { @obs count = 0; }
Counter.define('my-counter');
`,
          [framework()]
        ),
      ],
      [fileId("src/counter.ts")]
    ),
    emitted([
      ["dist/index.js", [fileId("src/counter.ts")], "bundle:counter"],
    ]),
    {
      "my-counter": component(
        "src/counter.ts",
        ["dist/index.js"],
        ["count"]
      ),
    }
  ),
  success(
    "namespace-decorator",
    "Namespace framework imports resolve decorators and base classes.",
    graph(
      [
        fileModule(
          "src/widget.ts",
          `
import * as webui from '@microsoft/webui-framework';
class Widget extends webui.WebUIElement { @webui.observable value = ''; }
Widget.define('my-widget');
`,
          [framework()]
        ),
      ],
      [fileId("src/widget.ts")]
    ),
    emitted([
      ["dist/index.js", [fileId("src/widget.ts")], "bundle:widget"],
    ]),
    {
      "my-widget": component(
        "src/widget.ts",
        ["dist/index.js"],
        ["value"]
      ),
    }
  ),
  success(
    "adapter-resolved-alias",
    "Package aliases use adapter package identity rather than path guessing.",
    graph(
      [
        fileModule(
          "src/alias-card.ts",
          `
import { observable, WebUIElement } from '#webui';
class AliasCard extends WebUIElement { @observable value = ''; }
AliasCard.define('alias-card');
`,
          [framework("#webui")]
        ),
      ],
      [fileId("src/alias-card.ts")]
    ),
    emitted([
      [
        "dist/index.js",
        [fileId("src/alias-card.ts")],
        "bundle:alias",
      ],
    ]),
    {
      "alias-card": component(
        "src/alias-card.ts",
        ["dist/index.js"],
        ["value"]
      ),
    }
  ),
  success(
    "re-export-chain",
    "Named and star re-exports resolve through adapter-provided edges.",
    graph(
      [
        fileModule(
          "src/framework-barrel.ts",
          "export { observable } from '@microsoft/webui-framework';",
          [framework()]
        ),
        fileModule(
          "src/app-barrel.ts",
          "export * from './framework-barrel.ts';",
          [resolved("./framework-barrel.ts", "src/framework-barrel.ts")]
        ),
        fileModule(
          "src/card.ts",
          `
import { observable } from './app-barrel.ts';
import { WebUIElement } from '@microsoft/webui-framework';
class Card extends WebUIElement { @observable title = ''; }
Card.define('my-card');
`,
          [
            resolved("./app-barrel.ts", "src/app-barrel.ts"),
            framework(),
          ]
        ),
      ],
      [fileId("src/card.ts")]
    ),
    emitted([
      [
        "dist/index.js",
        [
          fileId("src/framework-barrel.ts"),
          fileId("src/app-barrel.ts"),
          fileId("src/card.ts"),
        ],
        "bundle:barrel",
      ],
    ]),
    {
      "my-card": component(
        "src/card.ts",
        ["dist/index.js"],
        ["title"]
      ),
    }
  ),
  success(
    "inheritance-single",
    "Inherited and local keys are unioned exactly.",
    graph(
      [
        fileModule(
          "src/base.ts",
          `
import { observable, WebUIElement } from '@microsoft/webui-framework';
export class Base extends WebUIElement { @observable shared = ''; }
`,
          [framework()]
        ),
        fileModule(
          "src/derived.ts",
          `
import { observable } from '@microsoft/webui-framework';
import { Base } from './base.ts';
class Derived extends Base { @observable own = ''; }
Derived.define('my-derived');
`,
          [framework(), resolved("./base.ts", "src/base.ts")]
        ),
      ],
      [fileId("src/derived.ts")]
    ),
    emitted([
      [
        "dist/index.js",
        [fileId("src/base.ts"), fileId("src/derived.ts")],
        "bundle:derived",
      ],
    ]),
    {
      "my-derived": component(
        "src/derived.ts",
        ["dist/index.js"],
        ["own", "shared"]
      ),
    }
  ),
  success(
    "inheritance-multi",
    "Multiple inheritance hops remain deterministic.",
    graph(
      [
        fileModule(
          "src/a.ts",
          `
import { observable, WebUIElement } from '@microsoft/webui-framework';
export class A extends WebUIElement { @observable aKey = ''; }
`,
          [framework()]
        ),
        fileModule(
          "src/b.ts",
          `
import { observable } from '@microsoft/webui-framework';
import { A } from './a.ts';
export class B extends A { @observable bKey = ''; }
`,
          [framework(), resolved("./a.ts", "src/a.ts")]
        ),
        fileModule(
          "src/c.ts",
          `
import { observable } from '@microsoft/webui-framework';
import { B } from './b.ts';
class C extends B { @observable cKey = ''; }
C.define('my-c');
`,
          [framework(), resolved("./b.ts", "src/b.ts")]
        ),
      ],
      [fileId("src/c.ts")]
    ),
    emitted([
      [
        "dist/index.js",
        [fileId("src/a.ts"), fileId("src/b.ts"), fileId("src/c.ts")],
        "bundle:multi",
      ],
    ]),
    {
      "my-c": component(
        "src/c.ts",
        ["dist/index.js"],
        ["aKey", "bKey", "cKey"]
      ),
    }
  ),
  success(
    "code-splitting",
    "Final output membership identifies the shipped split chunk.",
    graph(
      [
        fileModule("src/index.ts", "import('./detail.ts');", [
          resolved("./detail.ts", "src/detail.ts", "dynamic"),
        ]),
        fileModule(
          "src/detail.ts",
          `
import { observable, WebUIElement } from '@microsoft/webui-framework';
class DetailPane extends WebUIElement { @observable item = null; }
DetailPane.define('detail-pane');
`,
          [framework()]
        ),
      ],
      [fileId("src/index.ts")]
    ),
    emitted([
      ["dist/index.js", [fileId("src/index.ts")], "bundle:index"],
      [
        "dist/chunks/detail.js",
        [fileId("src/detail.ts")],
        "bundle:detail",
      ],
    ]),
    {
      "detail-pane": component(
        "src/detail.ts",
        ["dist/chunks/detail.js"],
        ["item"]
      ),
    }
  ),
  success(
    "tree-shaking",
    "A defined class absent from every emitted output is excluded.",
    graph(
      [
        fileModule(
          "src/unused.ts",
          `
import { observable, WebUIElement } from '@microsoft/webui-framework';
class Unused extends WebUIElement { @observable value = ''; }
Unused.define('unused-card');
`,
          [framework()]
        ),
        fileModule("src/index.ts", "export const live = true;"),
      ],
      [fileId("src/index.ts")]
    ),
    emitted([
      ["dist/index.js", [fileId("src/index.ts")], "bundle:live"],
    ]),
    {}
  ),
  success(
    "shared-component",
    "One component may belong to one shared output used by multiple entries.",
    graph(
      [
        fileModule(
          "src/shared.ts",
          `
import { attr, WebUIElement } from '@microsoft/webui-framework';
export class Shared extends WebUIElement { @attr label = ''; }
Shared.define('shared-btn');
`,
          [framework()]
        ),
        fileModule("src/a.ts", "import './shared.ts';", [
          resolved("./shared.ts", "src/shared.ts"),
        ]),
        fileModule("src/b.ts", "import './shared.ts';", [
          resolved("./shared.ts", "src/shared.ts"),
        ]),
      ],
      [fileId("src/a.ts"), fileId("src/b.ts")]
    ),
    emitted([
      ["dist/a.js", [fileId("src/a.ts")], "bundle:a"],
      ["dist/b.js", [fileId("src/b.ts")], "bundle:b"],
      [
        "dist/chunks/shared.js",
        [fileId("src/shared.ts")],
        "bundle:shared",
      ],
    ]),
    {
      "shared-btn": component(
        "src/shared.ts",
        ["dist/chunks/shared.js"],
        ["label"]
      ),
    }
  ),
  success(
    "custom-elements-imported-class",
    "customElements.define resolves an imported class symbol.",
    graph(
      [
        fileModule(
          "src/card.ts",
          `
import { observable, WebUIElement } from '@microsoft/webui-framework';
export class Card extends WebUIElement { @observable value = ''; }
`,
          [framework()]
        ),
        fileModule(
          "src/register.ts",
          `
import { Card as ImportedCard } from './card.ts';
customElements.define('imported-card', ImportedCard);
`,
          [resolved("./card.ts", "src/card.ts")]
        ),
      ],
      [fileId("src/register.ts")]
    ),
    emitted([
      [
        "dist/index.js",
        [fileId("src/card.ts"), fileId("src/register.ts")],
        "bundle:imported",
      ],
    ]),
    {
      "imported-card": component(
        "src/card.ts",
        ["dist/index.js"],
        ["value"]
      ),
    }
  ),
  success(
    "class-expression",
    "Exported class expressions retain exact semantics.",
    graph(
      [
        fileModule(
          "src/expression.ts",
          `
import { observable, WebUIElement } from '@microsoft/webui-framework';
const ExpressionCard = class extends WebUIElement { @observable value = ''; };
ExpressionCard.define('expression-card');
`,
          [framework()]
        ),
      ],
      [fileId("src/expression.ts")]
    ),
    emitted([
      [
        "dist/index.js",
        [fileId("src/expression.ts")],
        "bundle:expression",
      ],
    ]),
    {
      "expression-card": component(
        "src/expression.ts",
        ["dist/index.js"],
        ["value"]
      ),
    }
  ),
  success(
    "unrelated-define-ignored",
    "Unrelated static define APIs do not produce false projection diagnostics.",
    graph(
      [
        fileModule(
          "src/schema.ts",
          `
class Schema {}
const dynamicName = getName();
Schema.define(dynamicName);
`
        ),
      ],
      [fileId("src/schema.ts")]
    ),
    emitted([
      ["dist/index.js", [fileId("src/schema.ts")], "bundle:schema"],
    ]),
    {}
  ),
  success(
    "virtual-source",
    "Virtual graph inputs are provenance-only and never re-read from disk.",
    graph(
      [
        virtualModule("\0virtual:polyfill", undefined),
        fileModule("src/index.ts", "import 'virtual:polyfill';", [
          {
            specifier: "virtual:polyfill",
            resolvedId: "\0virtual:polyfill",
            external: false,
            kind: "static",
          },
        ]),
      ],
      [fileId("src/index.ts")]
    ),
    emitted([
      [
        "dist/index.js",
        [fileId("src/index.ts"), "\0virtual:polyfill"],
        "bundle:virtual",
      ],
    ]),
    {}
  ),
  failure(
    "duplicate-tag-error",
    "Two shipped WebUI classes cannot own one tag.",
    graph(
      [
        fileModule(
          "src/a.ts",
          `
import { WebUIElement } from '@microsoft/webui-framework';
class A extends WebUIElement {}
A.define('duplicate-card');
`,
          [framework()]
        ),
        fileModule(
          "src/b.ts",
          `
import { WebUIElement } from '@microsoft/webui-framework';
class B extends WebUIElement {}
B.define('duplicate-card');
`,
          [framework()]
        ),
      ],
      [fileId("src/a.ts"), fileId("src/b.ts")]
    ),
    emitted([
      [
        "dist/index.js",
        [fileId("src/a.ts"), fileId("src/b.ts")],
        "bundle:duplicate",
      ],
    ]),
    ["PROJ-C010"]
  ),
  failure(
    "unresolvable-base-error",
    "A likely WebUI class with unresolved inheritance cannot prove exact keys.",
    graph(
      [
        fileModule(
          "src/derived.ts",
          `
import { observable } from '@microsoft/webui-framework';
import { MissingBase } from './missing.ts';
class Derived extends MissingBase { @observable value = ''; }
Derived.define('derived-card');
`,
          [framework()]
        ),
      ],
      [fileId("src/derived.ts")]
    ),
    emitted([
      [
        "dist/index.js",
        [fileId("src/derived.ts")],
        "bundle:missing-base",
      ],
    ]),
    ["PROJ-C005"]
  ),
  failure(
    "dynamic-tag-error",
    "A proven WebUI class requires a literal component tag.",
    graph(
      [
        fileModule(
          "src/dynamic.ts",
          `
import { WebUIElement } from '@microsoft/webui-framework';
const tag = 'dynamic-card';
class DynamicCard extends WebUIElement {}
DynamicCard.define(tag);
`,
          [framework()]
        ),
      ],
      [fileId("src/dynamic.ts")]
    ),
    emitted([
      [
        "dist/index.js",
        [fileId("src/dynamic.ts")],
        "bundle:dynamic",
      ],
    ]),
    ["PROJ-C008"]
  ),
  {
    ...failure(
      "missing-output-bytes-error",
      "Physical emitted outputs require exact bytes.",
      contactGraph,
      emitted([
        [
          "dist/index.js",
          [fileId("src/index.ts"), contactId],
          "bundle:contact",
        ],
      ]),
      ["PROJ-C014"]
    ),
    outputContents: new Map(),
  },
  rustCase(
    "stale-input-error",
    "The Rust consumer rejects source hashes that no longer match disk."
  ),
  rustCase(
    "missing-coverage-error",
    "The Rust consumer rejects a compiled scripted tag absent from all fragments."
  ),
  rustCase(
    "external-bundle",
    "The Rust consumer merges disjoint application and external bundle fragments."
  ),
];
