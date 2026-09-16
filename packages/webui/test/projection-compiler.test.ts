// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import * as path from "node:path";
import { describe, test } from "node:test";
import {
  compileProjection,
  computeBuildId,
  ProjectionError,
  serializeManifestCanonical,
  validateManifestSchema,
} from "@microsoft/webui/projection.js";
import type {
  AdapterContext,
  AttributeEntry,
  ModuleNode,
  ResolvedImport,
} from "@microsoft/webui/projection.js";

const ROOT = path.resolve(".webui-projection-unit");

function id(relative: string): string {
  return path.join(ROOT, relative);
}

function frameworkEdge(): ResolvedImport {
  return {
    specifier: "@microsoft/webui-framework",
    resolvedId: undefined,
    external: true,
    kind: "static",
    packageName: "@microsoft/webui-framework",
  };
}

function context(modules: ReadonlyArray<ModuleNode>): AdapterContext {
  const outputId = id("dist/index.js");
  return {
    graph: {
      modules: new Map(modules.map((module) => [module.id, module])),
      entries: [modules[modules.length - 1]!.id],
    },
    membership: {
      outputs: new Map([
        [outputId, new Set(modules.map((module) => module.id))],
      ]),
    },
    outputContents: new Map([[outputId, "compiled-output"]]),
    rootDir: ROOT,
    manifestPath: id("dist/webui-projection.json"),
    bundlerName: "test",
    bundlerVersion: "1.0.0",
  };
}

function compileCard(body: string) {
  return compileProjection(context([{
    id: id("src/card.ts"),
    kind: "file",
    source: `
import { attr, observable, WebUIElement } from '@microsoft/webui-framework';
${body}
Card.define('test-card');
`,
    imports: [frameworkEdge()],
  }]));
}

describe("projection compiler semantics", () => {
  test("preserves exact attr aliases and modes without evaluating defaults", async () => {
    const manifest = await compileCard(`
class Card extends WebUIElement {
  @attr({ attribute: 'aria-describedby' }) ariaDescribedby = (() => { throw new Error('must not execute'); })();
  @attr ariaLabelledBy = '';
  @attr({ attribute: 'custom-attribute' }) unrelated = '';
  @attr({ 'attribute': 'custom-boolean', 'mode': 'boolean' }) enabled = false;
  @attr({ attribute: 'aria-selected' }) stringFlag = 'false';
  @attr({ attribute: \`static-alias\`, mode: \`boolean\` }) templateLiteral = false;
  @attr() factory = '';
  @attr({}) emptyOptions = '';
  @attr __proto__ = '';
  @attr 𐐀 = '';
  @attr Ａ = '';
  @observable stateOnly = '';
}
`);
    const attributes = manifest.components["test-card"]?.attributes;
    assert.deepEqual(attributes, Object.fromEntries([
      ["__proto__", { property: "__proto__", mode: 0 }],
      ["aria-describedby", { property: "ariaDescribedby", mode: 0 }],
      ["aria-labelledby", { property: "ariaLabelledBy", mode: 0 }],
      ["aria-selected", { property: "stringFlag", mode: 0 }],
      ["custom-attribute", { property: "unrelated", mode: 0 }],
      ["custom-boolean", { property: "enabled", mode: 1 }],
      ["empty-options", { property: "emptyOptions", mode: 0 }],
      ["factory", { property: "factory", mode: 0 }],
      ["static-alias", { property: "templateLiteral", mode: 1 }],
      ["Ａ", { property: "Ａ", mode: 0 }],
      ["𐐀", { property: "𐐀", mode: 0 }],
    ]));
    assert.deepEqual(Object.keys(attributes!), [
      "__proto__", "aria-describedby", "aria-labelledby", "aria-selected",
      "custom-attribute", "custom-boolean", "empty-options", "factory",
      "static-alias", "Ａ", "𐐀",
    ]);
    assert.deepEqual(validateManifestSchema(manifest), []);
    assert.deepEqual(
      manifest.components["test-card"]?.hydrationKeys,
      manifest.components["test-card"]?.navigationKeys
    );
    assert.ok(manifest.components["test-card"]?.hydrationKeys.includes("stateOnly"));
  });

  test("resolves attr aliases through namespaces and re-exports with inheritance", async () => {
    const barrelId = id("src/decorators.ts");
    const baseId = id("src/base.ts");
    const moduleId = id("src/card.ts");
    const modules: ModuleNode[] = [
      {
        id: barrelId,
        kind: "file",
        source: "export { attr as attribute, WebUIElement } from '@microsoft/webui-framework';",
        imports: [frameworkEdge()],
      },
      {
        id: baseId,
        kind: "file",
        source: `
import * as ui from './decorators.js';
const reflected = ui.attribute;
export class Base extends ui.WebUIElement {
  @reflected({ attribute: 'base-alias', mode: 'boolean' }) replaced = false;
  @reflected({ attribute: 'inherited-alias' }) retained = '';
  @reflected({ attribute: 'first-attribute' }) first = '';
}
`,
        imports: [{ specifier: "./decorators.js", resolvedId: barrelId, external: false, kind: "static" }],
      },
      {
        id: moduleId,
        kind: "file",
        source: `
import { Base } from './base.js';
import { attribute } from './decorators.js';
import { observable } from '@microsoft/webui-framework';
class Card extends Base {
  @attribute({ attribute: 'derived-alias' }) replaced = '';
  @observable retained = '';
  @attribute({ attribute: 'second-attribute', mode: 'boolean' }) second = false;
}
Card.define('test-card');
`,
        imports: [
          { specifier: "./base.js", resolvedId: baseId, external: false, kind: "static" },
          { specifier: "./decorators.js", resolvedId: barrelId, external: false, kind: "static" },
          frameworkEdge(),
        ],
      },
    ];
    const ctx = context(modules);
    const manifest = await compileProjection(ctx);
    assert.deepEqual(manifest.components["test-card"]?.attributes, {
      "base-alias": { property: "replaced", mode: 1 },
      "derived-alias": { property: "replaced", mode: 0 },
      "first-attribute": { property: "first", mode: 0 },
      "inherited-alias": { property: "retained", mode: 0 },
      "second-attribute": { property: "second", mode: 1 },
    });
    const reversed = await compileProjection({
      ...ctx,
      graph: { ...ctx.graph, modules: new Map([...ctx.graph.modules].reverse()) },
    });
    assert.equal(serializeManifestCanonical(manifest), serializeManifestCanonical(reversed));
  });

  const attributeOverrides: ReadonlyArray<readonly [
    string, string, Readonly<Record<string, AttributeEntry>>, readonly string[],
  ]> = [
    ["explicit aliases", `
class Base extends WebUIElement { @attr({ attribute: 'shared-name' }) zBase = ''; }
class Card extends Base { @attr({ attribute: 'shared-name', mode: 'boolean' }) aDerived = false; }
`, { "shared-name": { property: "aDerived", mode: 1 } }, ["aDerived", "zBase"]],
    ["own explicit aliases", `
class Card extends WebUIElement {
  @attr({ attribute: 'shared-name' }) zFirst = '';
  @attr({ attribute: 'shared-name' }) aSecond = '';
}
`, { "shared-name": { property: "aSecond", mode: 0 } }, ["aSecond", "zFirst"]],
    ["default kebab name", `
class Base extends WebUIElement { @attr displayValue = ''; }
class Card extends Base { @attr({ attribute: 'display-value' }) unrelated = ''; }
`, { "display-value": { property: "unrelated", mode: 0 } }, ["displayValue", "unrelated"]],
    ["ARIA default suffix", `
class Base extends WebUIElement { @attr ariaDescribedBy = ''; }
class Card extends Base { @attr ariaDescribedby = ''; }
`, { "aria-describedby": { property: "ariaDescribedby", mode: 0 } }, ["ariaDescribedBy", "ariaDescribedby"]],
    ["shadowed registry aliases", `
class Base extends WebUIElement { @attr({ attribute: 'old-name' }) value = ''; }
class Card extends Base {
  @attr({ attribute: 'new-name' }) value = '';
  @attr({ attribute: 'old-name' }) unrelated = '';
}
`, {
      "new-name": { property: "value", mode: 0 },
      "old-name": { property: "unrelated", mode: 0 },
    }, ["unrelated", "value"]],
    ["same-alias mode override", `
class Base extends WebUIElement { @attr({ attribute: 'same-name', mode: 'boolean' }) value = false; }
class Card extends Base { @attr({ attribute: 'same-name' }) value = ''; }
`, { "same-name": { property: "value", mode: 0 } }, ["value"]],
    ["default and explicit equivalent names", `
class Base extends WebUIElement { @attr({ mode: 'boolean' }) ariaExpanded = false; }
class Card extends Base { @attr({ attribute: 'aria-expanded' }) ariaExpanded = ''; }
`, { "aria-expanded": { property: "ariaExpanded", mode: 0 } }, ["ariaExpanded"]],
  ];
  for (const [name, body, expected, keys] of attributeOverrides) {
    test(`resolves actual attribute registration precedence: ${name}`, async () => {
      const manifest = await compileCard(body);
      const component = manifest.components["test-card"]!;
      assert.deepEqual(component.attributes, expected);
      assert.deepEqual(component.hydrationKeys, keys);
      assert.deepEqual(component.navigationKeys, keys);
      assert.deepEqual(validateManifestSchema(manifest), []);
    });
  }

  test("preserves inherited old/new aliases with independent modes for one property", async () => {
    const manifest = await compileCard(`
class Base extends WebUIElement {
  @attr({ attribute: 'old-expanded', mode: 'boolean' }) expanded = false;
}
class Card extends Base {
  @attr({ attribute: 'new-expanded' }) expanded = '';
}
`);
    assert.deepEqual(manifest.components["test-card"]?.attributes, {
      "new-expanded": { property: "expanded", mode: 0 },
      "old-expanded": { property: "expanded", mode: 1 },
    });
    assert.deepEqual(manifest.components["test-card"]?.hydrationKeys, ["expanded"]);
    assert.deepEqual(validateManifestSchema(manifest), []);
  });

  test("resolves every irregular default name and later own alias registration", async () => {
    const properties = [
      "accessKey", "autoCapitalize", "contentEditable", "crossOrigin", "dirName",
      "fetchPriority", "formAction", "formEnctype", "formMethod", "formNoValidate",
      "formTarget", "inputMode", "isMap", "maxLength", "minLength", "noModule",
      "noValidate", "readOnly", "referrerPolicy", "tabIndex", "useMap",
    ];
    const body = properties.map((property) => `
  @attr ${property} = '';
  @attr({ attribute: '${property.toLowerCase()}' }) ${property}Alias = '';
`).join("");
    const manifest = await compileCard(`class Card extends WebUIElement { ${body} }`);
    assert.deepEqual(
      manifest.components["test-card"]?.attributes,
      Object.fromEntries(properties.map((property) => [
        property.toLowerCase(), { property: `${property}Alias`, mode: 0 },
      ]))
    );
    assert.deepEqual(
      manifest.components["test-card"]?.hydrationKeys,
      properties.flatMap((property) => [property, `${property}Alias`]).sort()
    );
  });

  test("matches bottom-up attr registration and observable registry semantics", async () => {
    const manifest = await compileCard(`
class Card extends WebUIElement {
  @attr({ attribute: 'final-alias' })
  @attr({ attribute: 'first-alias', mode: 'boolean' })
  multiple = '';
  @observable
  @attr({ attribute: 'retained-alias' })
  retained = '';
  @attr({ attribute: 'same-alias', mode: 'boolean' })
  @attr({ attribute: 'same-alias' })
  same = false;
}
`);
    assert.deepEqual(manifest.components["test-card"]?.attributes, {
      "final-alias": { property: "multiple", mode: 0 },
      "first-alias": { property: "multiple", mode: 1 },
      "retained-alias": { property: "retained", mode: 0 },
      "same-alias": { property: "same", mode: 1 },
    });
  });

  test("omits metadata for classes without attr properties", async () => {
    const manifest = await compileCard("class Card extends WebUIElement { @observable value = ''; }");
    assert.equal(Object.hasOwn(manifest.components["test-card"]!, "attributes"), false);
    assert.deepEqual(validateManifestSchema(manifest), []);
  });

  const invalidOptions = [
    "options", "{ ...options }", "{ ['attribute']: 'alias' }",
    "{ attribute }", "{ get attribute() { return 'alias'; } }",
    "{ attribute: '' }", "{ attribute: alias }", "{ attribute: `${alias}` }",
    "{ attribute: 'a' + 'b' }", "{ attribute: null }", "{ attribute: undefined }",
    "{ mode: mode }", "{ mode: 'string' }", "{ mode: true }",
    "{ mode: 1 }", "{ unknown: 'alias' }", "{ attribute: 'alias' }, {}",
    "{ attribute: 'two names' }", "{ attribute: 'bad=name' }",
    "{ attribute: 'bad<name' }", "{ attribute: 'bad>name' }",
    "{ attribute: 'bad/name' }", "{ attribute: 'bad`name' }",
    "{ attribute: 'bad\\tname' }", "{ attribute: 'bad\\u0000name' }",
    "{ attribute: 'bad\"name' }", '{ attribute: "bad\'name" }',
  ];
  for (const options of invalidOptions) {
    test(`rejects unsupported attr options: ${options}`, async () => {
      await assert.rejects(
        compileCard(`class Card extends WebUIElement { @attr(${options}) value = ''; }`),
        (error: unknown) => {
          assert.ok(error instanceof ProjectionError);
          assert.deepEqual(error.diagnostics.map((diagnostic) => diagnostic.code), ["PROJ-C007"]);
          assert.ok(error.diagnostics[0]?.help?.includes("literal"));
          assert.ok(error.diagnostics[0]?.location?.includes("card.ts"));
          return true;
        }
      );
    });
  }

  test("uses adapter-resolved targets instead of reconstructing extensions", async () => {
    const baseId = id("src/base.ts");
    const derivedId = id("src/derived.ts");
    const manifest = await compileProjection(
      context([
        {
          id: baseId,
          kind: "file",
          source: `
import { observable, WebUIElement } from '@microsoft/webui-framework';
export class Base extends WebUIElement { @observable baseValue = ''; }
`,
          imports: [frameworkEdge()],
        },
        {
          id: derivedId,
          kind: "file",
          source: `
import { Base } from './base.js';
class Derived extends Base {}
Derived.define('resolved-card');
`,
          imports: [
            {
              specifier: "./base.js",
              resolvedId: baseId,
              external: false,
              kind: "static",
            },
          ],
        },
      ])
    );

    assert.deepEqual(
      manifest.components["resolved-card"]?.hydrationKeys,
      ["baseValue"]
    );
    assert.deepEqual(
      manifest.components["resolved-card"]?.navigationKeys,
      ["baseValue"]
    );
  });

  test("ignores known non-framework property decorators", async () => {
    const cardId = id("src/decorated.ts");
    const manifest = await compileProjection(
      context([
        {
          id: cardId,
          kind: "file",
          source: `
import { observable, WebUIElement } from '@microsoft/webui-framework';
function localDecorator(_target: object, _name: string): void {}
class Decorated extends WebUIElement {
  @localDecorator
  @observable value = '';
}
Decorated.define('decorated-card');
`,
          imports: [frameworkEdge()],
        },
      ])
    );

    assert.deepEqual(
      manifest.components["decorated-card"]?.hydrationKeys,
      ["value"]
    );
    assert.deepEqual(
      manifest.components["decorated-card"]?.navigationKeys,
      ["value"]
    );
  });

  test("ignores a locally shadowed customElements registry", async () => {
    const moduleId = id("src/local-registry.ts");
    const manifest = await compileProjection(
      context([
        {
          id: moduleId,
          kind: "file",
          source: `
import { WebUIElement } from '@microsoft/webui-framework';
class Card extends WebUIElement {}
const customElements = { define() {} };
customElements.define('not-global-card', Card);
`,
          imports: [frameworkEdge()],
        },
      ])
    );

    assert.deepEqual(manifest.components, {});
  });

  test("recognizes define calls separated from arguments by comments", async () => {
    const moduleId = id("src/commented-define.ts");
    const manifest = await compileProjection(
      context([
        {
          id: moduleId,
          kind: "file",
          source: `
import { WebUIElement } from '@microsoft/webui-framework';
class Card extends WebUIElement {}
Card.define /* keep this comment */ ('commented-card');
`,
          imports: [frameworkEdge()],
        },
      ])
    );

    assert.deepEqual(
      manifest.components["commented-card"]?.hydrationKeys,
      []
    );
  });

  test("recognizes define calls after interpolated template literals", async () => {
    const moduleId = id("src/template-before-define.ts");
    const manifest = await compileProjection(
      context([
        {
          id: moduleId,
          kind: "file",
          source: `
import { WebUIElement } from '@microsoft/webui-framework';
const label = \`card-\${String(1)}\`;
class Card extends WebUIElement {}
Card.define('template-card');
void label;
`,
          imports: [frameworkEdge()],
        },
      ])
    );

    assert.deepEqual(
      manifest.components["template-card"]?.hydrationKeys,
      []
    );
  });

  test("resolves immutable class and decorator aliases", async () => {
    const moduleId = id("src/aliases.ts");
    const manifest = await compileProjection(
      context([
        {
          id: moduleId,
          kind: "file",
          source: `
import { WebUIElement, observable } from '@microsoft/webui-framework';
const obs = observable;
class Card extends WebUIElement {
  @obs value = '';
}
const Alias = Card;
Alias.define('aliased-card');
`,
          imports: [frameworkEdge()],
        },
      ])
    );

    assert.deepEqual(
      manifest.components["aliased-card"]?.hydrationKeys,
      ["value"]
    );
  });

  test("rejects mutable class-expression bindings", async () => {
    const moduleId = id("src/mutable-class.ts");
    await assert.rejects(
      () =>
        compileProjection(
          context([
            {
              id: moduleId,
              kind: "file",
              source: `
import { WebUIElement, observable } from '@microsoft/webui-framework';
let Card = class extends WebUIElement {
  @observable stale = '';
};
Card.define('mutable-card');
`,
              imports: [frameworkEdge()],
            },
          ])
        ),
      /PROJ-C009/
    );
  });
});

describe("projection manifest hashing", () => {
  test("covers empty and populated attribute declarations after entry closures", () => {
    const params = {
      producerName: "@microsoft/webui/projection.js",
      producerVersion: "0.0.18",
      adapterName: "esbuild",
      adapterBundler: "esbuild@0.28.1",
      root: "..",
      analysisHash: `sha256:${"1".repeat(64)}`,
      sortedInputs: [["src/a.ts", `sha256:${"2".repeat(64)}`]] as const,
      sortedOutputs: [["dist/a.js", `sha256:${"3".repeat(64)}`]] as const,
      sortedComponents: [
        ["a-card", "src/a.ts", ["dist/a.js"], ["displayValue"], ["displayValue", "é"]],
      ] as const,
      sortedEntryClosures: [["dist/a.js", []]] as const,
    };
    const empty = computeBuildId(params);
    assert.equal(computeBuildId({ ...params, sortedComponentAttributes: [] }), empty);
    const hash = (definition: readonly [string, string, string, 0 | 1]) =>
      computeBuildId({ ...params, sortedComponentAttributes: [definition] });
    const original = hash(["a-card", "arbitrary-alias", "displayValue", 0]);
    assert.equal(
      original,
      "sha256:021035acc1709ea71c8b90c4112ce1f9598a404b9efb9db0f27fb9d355ca43e8"
    );
    assert.notEqual(original, empty);
    assert.notEqual(original, hash(["a-card", "arbitrary-alias", "displayValue", 1]));
    assert.notEqual(original, hash(["a-card", "display-value", "displayValue", 0]));
    assert.notEqual(original, hash(["a-card", "other-alias", "displayValue", 0]));
    assert.notEqual(original, hash(["a-card", "arbitrary-alias", "é", 0]));
    assert.notEqual(original, hash(["b-card", "arbitrary-alias", "displayValue", 0]));
    assert.notEqual(original, computeBuildId({
      ...params,
      sortedComponentAttributes: [
        ["a-card", "arbitrary-alias", "displayValue", 0],
        ["a-card", "old-alias", "displayValue", 1],
      ],
    }));
  });

  test("compiler includes sorted component attributes in its build ID", async () => {
    const manifest = await compileCard(`
class Card extends WebUIElement {
  @attr({ attribute: 'aria-describedby' }) unrelated = '';
  @attr({ mode: 'boolean' }) enabled = false;
  @attr({ attribute: '2' }) numericTwo = '';
  @attr({ attribute: '10' }) numericTen = '';
}
Card.define('a-card');
`);
    const params = {
      producerName: manifest.producer.name,
      producerVersion: manifest.producer.version,
      adapterName: manifest.adapter.name,
      adapterBundler: manifest.adapter.bundler,
      root: manifest.root,
      analysisHash: manifest.analysisHash,
      sortedInputs: Object.entries(manifest.inputs),
      sortedOutputs: Object.entries(manifest.outputs),
      sortedComponents: Object.entries(manifest.components).map(([tag, entry]) =>
        [tag, entry.module, entry.outputs, entry.hydrationKeys, entry.navigationKeys] as const
      ),
    };
    assert.equal(manifest.buildId, computeBuildId({
      ...params,
      sortedComponentAttributes: [
        ["a-card", "10", "numericTen", 0],
        ["a-card", "2", "numericTwo", 0],
        ["a-card", "aria-describedby", "unrelated", 0],
        ["a-card", "enabled", "enabled", 1],
        ["test-card", "10", "numericTen", 0],
        ["test-card", "2", "numericTwo", 0],
        ["test-card", "aria-describedby", "unrelated", 0],
        ["test-card", "enabled", "enabled", 1],
      ],
    }));
    assert.notEqual(manifest.buildId, computeBuildId(params));
  });

  test("canonicalizes attribute maps and entries and omits empty maps", async () => {
    const manifest = await compileCard("class Card extends WebUIElement { @observable value = ''; }");
    const component = manifest.components["test-card"]!;
    const attributes = Object.fromEntries<AttributeEntry>([
      ["𐐀", { mode: 1, property: "value" }],
      ["Ａ", { mode: 0, property: "value" }],
      ["__proto__", { mode: 0, property: "value" }],
      ["2", { mode: 1, property: "value" }],
      ["10", { mode: 0, property: "value" }],
    ]);
    const json = serializeManifestCanonical({
      ...manifest,
      components: { "test-card": { ...component, attributes } },
    });
    assert.ok(json.includes('"attributes":{"10":{"property":"value","mode":0},"2":{"property":"value","mode":1},"__proto__":{"property":"value","mode":0},"Ａ":{"property":"value","mode":0},"𐐀":{"property":"value","mode":1}}'));
    assert.deepEqual(validateManifestSchema(JSON.parse(json)), []);
    assert.equal(
      serializeManifestCanonical({
        ...manifest,
        components: { "test-card": { ...component, attributes: {} } },
      }),
      serializeManifestCanonical(manifest)
    );
  });

  test("validates attribute metadata without accepting guessed or unknown fields", async () => {
    const manifest = await compileCard(`
class Card extends WebUIElement {
  @attr({ attribute: 'custom-alias' }) value = '';
  @attr z = '';
}
`);
    assert.deepEqual(validateManifestSchema(manifest), []);
    const component = manifest.components["test-card"]!;
    for (const attributes of [
      {}, null, [], { alias: {} }, { alias: { property: "value", mode: true } },
      { alias: { property: "value", mode: 2 } }, { alias: { mode: 0 } },
      { alias: { property: "", mode: 0 } }, { alias: { property: null, mode: 0 } },
      { alias: { property: "missing", mode: 0 } },
      { alias: { property: "value", mode: 0, unknown: "x" } },
      { alias: { property: "value", mode: 0, attribute: "old-shape" } },
      { z: { property: "z", mode: 0 }, alias: { property: "value", mode: 0 } },
    ]) {
      assert.deepEqual(validateManifestSchema({
        ...manifest,
        components: { "test-card": { ...component, attributes } },
      }), ["PROJ-M009"], JSON.stringify(attributes));
    }
    for (const attribute of ["", "two words", "bad\tname", "\u0000", '"', "'", "<", ">", "/", "=", "`"]) {
      assert.deepEqual(validateManifestSchema({
        ...manifest,
        components: {
          "test-card": {
            ...component,
            attributes: { [attribute]: { property: "value", mode: 0 } },
          },
        },
      }), ["PROJ-M009"], JSON.stringify(attribute));
    }
    assert.deepEqual(validateManifestSchema({
      ...manifest,
      components: {
        "test-card": {
          ...component,
          attributes: {
            "new-alias": { property: "value", mode: 0 },
            "old-alias": { property: "value", mode: 1 },
          },
        },
      },
    }), []);
  });

  test("matches the cross-language canonical build-ID vector", () => {
    const buildId = computeBuildId({
      producerName: "@microsoft/webui/projection.js",
      producerVersion: "0.0.18",
      adapterName: "esbuild",
      adapterBundler: "esbuild@0.28.1",
      root: "..",
      analysisHash: `sha256:${"1".repeat(64)}`,
      sortedInputs: [["src/a.ts", `sha256:${"2".repeat(64)}`]],
      sortedOutputs: [["dist/a.js", `sha256:${"3".repeat(64)}`]],
      sortedComponents: [
        [
          "a-card",
          "src/a.ts",
          ["dist/a.js"],
          ["displayValue"],
          ["displayValue", "é"],
        ],
      ],
    });

    assert.equal(
      buildId,
      "sha256:439764b5adbf055a080369870085bc81aed17ebba83a05c0e12fd94b1c9808cb"
    );
  });

  test("component output membership changes the build ID", () => {
    const common = {
      producerName: "@microsoft/webui/projection.js",
      producerVersion: "0.0.18",
      adapterName: "esbuild",
      adapterBundler: "esbuild@0.28.1",
      root: "..",
      analysisHash: `sha256:${"1".repeat(64)}`,
      sortedInputs: [] as const,
      sortedOutputs: [
        ["dist/a.js", `sha256:${"2".repeat(64)}`],
        ["dist/b.js", `sha256:${"3".repeat(64)}`],
      ] as const,
    };
    const first = computeBuildId({
      ...common,
      sortedComponents: [
        [
          "a-card",
          "src/a.ts",
          ["dist/a.js"],
          ["value"],
          ["value"],
        ],
      ],
    });
    const second = computeBuildId({
      ...common,
      sortedComponents: [
        [
          "a-card",
          "src/a.ts",
          ["dist/b.js"],
          ["value"],
          ["value"],
        ],
      ],
    });

    assert.notEqual(first, second);
  });

  test("canonical serialization fixes top-level and map order", () => {
    const json = serializeManifestCanonical({
      schema: "webui.state-projection/v1",
      producer: {
        name: "@microsoft/webui/projection.js",
        version: "1.0.0",
      },
      adapter: { name: "test", bundler: "test@1.0.0" },
      root: "..",
      analysisHash: `sha256:${"1".repeat(64)}`,
      buildId: `sha256:${"2".repeat(64)}`,
      outputs: {
        "dist/z.js": `sha256:${"3".repeat(64)}`,
        "dist/a.js": `sha256:${"4".repeat(64)}`,
      },
      inputs: {
        "src/z.ts": `sha256:${"5".repeat(64)}`,
        "src/a.ts": `sha256:${"6".repeat(64)}`,
      },
      components: {},
    });

    assert.ok(json.indexOf('"dist/a.js"') < json.indexOf('"dist/z.js"'));
    assert.ok(json.indexOf('"src/a.ts"') < json.indexOf('"src/z.ts"'));
  });

  test("rejects virtual hashes on physical disk paths", () => {
    const errors = validateManifestSchema({
      schema: "webui.state-projection/v1",
      producer: {
        name: "@microsoft/webui/projection.js",
        version: "1.0.0",
      },
      adapter: { name: "test", bundler: "test@1.0.0" },
      root: "..",
      analysisHash: `sha256:${"1".repeat(64)}`,
      buildId: `sha256:${"2".repeat(64)}`,
      outputs: { "dist/index.js": "virtual" },
      inputs: {},
      components: {},
    });

    assert.deepEqual(errors, ["PROJ-S004"]);
  });
});
