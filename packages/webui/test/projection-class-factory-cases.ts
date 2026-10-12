// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import * as path from "node:path";
import type { AdapterContext, ProjectionManifest, ResolvedImport } from "@microsoft/webui/projection.js";

interface FactoryCase {
  readonly name: string;
  readonly source: string;
  readonly base?: string;
  readonly keys?: readonly string[];
  readonly fields?: string;
  readonly before?: string;
  readonly definition?: string;
}

const namedFactory = `
export function mix<T extends new (...args: any[]) => WebUIElement>(
  Base: T, configuration: { value: string }
) {
  class Mixed extends Base {
    @attr inherited = configuration.value;
    ordinary = 0;
    connectedCallback() { super.connectedCallback(); this.ordinary += 1; }
  }
  return Mixed;
}`;

export const CLASS_FACTORY_CASES: readonly FactoryCase[] = [
  {
    name: "named reexport, namespace and immutable alias",
    source: namedFactory,
    base: "mix(Base, { value: 'initial' })",
    keys: ["base", "inherited", "own"],
  },
  {
    name: "inherited-only state",
    source: namedFactory,
    base: "factories.mix(WebUIElement, { value: 'initial' })",
    fields: "",
    keys: ["inherited"],
  },
  {
    name: "repeated factory retains the supplied base",
    source: namedFactory,
    base: "mix(mix(Base, { value: 'inner' }), { value: 'outer' })",
    keys: ["base", "inherited", "own"],
  },
  {
    name: "direct returned class expression",
    source: "export function mix(Base) { return class extends Base {}; }",
    base: "mix(Base)",
    keys: ["base", "own"],
  },
  {
    name: "empty state",
    source: "export function mix(Base) { return class extends Base {}; }",
    fields: "",
    keys: [],
  },
  {
    name: "inline base class",
    source: namedFactory,
    base: "mix(class extends Base {}, { value: '' })",
    keys: ["base", "inherited", "own"],
  },
  {
    name: "nonfirst constructor parameter",
    source: "export function mix(config, Base) { return class Named extends Base {}; }",
    base: "mix({ value: 'initial' }, Base)",
    keys: ["base", "own"],
  },
  ...[
    "let other; ({ other = mix } = {});",
    "for (const { factory = mix } of [{}]) { void factory; }",
    "for (const { mix: selected } of []) { void selected; }",
    "for (const [selected = mix] of []) { void selected; }",
    "for (const { mix } of []) { void mix; }",
    "function local(mix) { mix = null; }",
    "{ let mix; mix = null; }",
    "try {} catch (mix) { mix = null; }",
    "class Holder { static { var mix; mix = replacement; } }",
    "function local(mix = replacement) { var mix; mix = null; }",
  ].map((statement) => ({
    name: `preserves factory identity around ${statement}`,
    source: `${namedFactory}\n${statement}`,
    base: "mix(Base, { value: 'initial' })",
    keys: ["base", "inherited", "own"],
  })),
  ...[
    {
      name: "conditional return",
      source: "export function mix(Base) { if (flag) return Base; return class extends Base {}; }",
    },
    {
      name: "base parameter reassignment",
      source: "export function mix(Base) { Base = Other; return class extends Base {}; }",
    },
    ...[
      "mix = replacement;",
      "export function replace() { mix = replacement; }",
      "({ factory: mix } = replacement);",
      "({ mix } = replacement);",
      "[mix] = replacements;",
      "mix++;",
      "for (mix of replacements) {}",
      "for ({ factory: mix } of replacements) {}",
      "for (var mix of replacements) {}",
      "for (const { factory = (mix = replacement) } of [{}]) {}",
      "(mix as any) = replacement;",
      "mix! = replacement;",
      "(<any>mix) = replacement;",
      "(mix satisfies unknown) = replacement;",
      "[(mix as any)] = replacements;",
      "function replace(value = (mix = replacement)) { var mix; } replace();",
      "class Effects { [(mix = replacement, 'method')](mix) {} }",
    ].map((statement) => ({
      name: `factory mutation: ${statement}`,
      source: `${namedFactory}\n${statement}`,
    })),
    {
      name: "dynamic base",
      source: "export function mix(Base, config) { return class extends config.base {}; }",
    },
    {
      name: "shadowed decorator parameter",
      source: "export function mix(Base, observable) { class Mixed extends Base { @observable value = ''; } return Mixed; }",
    },
    {
      name: "shadowed decorator class name",
      source: "export function mix(Base) { class observable extends Base { @observable value = ''; } return observable; }",
    },
    {
      name: "class decorator",
      source: "export function mix(Base) { @replace class Mixed extends Base {} return Mixed; }",
    },
    {
      name: "static block",
      source: "export function mix(Base) { return class extends Base { static { replace(); } }; }",
    },
    {
      name: "static initializer",
      source: "export function mix(Base) { return class extends Base { static value = replace(); }; }",
    },
    {
      name: "default parameter",
      source: "export function mix(Base = replacement()) { return class extends Base {}; }",
    },
    {
      name: "rest parameter",
      source: "export function mix(...Base) { return class extends Base {}; }",
    },
    {
      name: "destructured parameter",
      source: "export function mix({ Base }) { return class extends Base {}; }",
    },
    {
      name: "async factory",
      source: "export async function mix(Base) { return class extends Base {}; }",
    },
    {
      name: "generator factory",
      source: "export function* mix(Base) { return class extends Base {}; }",
    },
    {
      name: "recursive factory",
      source: "export function mix(Base) { return mix(Base); }",
    },
    {
      name: "unknown factory",
      source: "export function mix(Base) { return externalFactory(Base); }",
    },
    {
      name: "class called as factory",
      source: "export class mix extends WebUIElement {}",
    },
  ],
  ...["mix()", "mix(...bases)", "mix?.(WebUIElement)"].map((base) => ({
    name: `unproven call: ${base}`, source: namedFactory, base,
  })),
  ...[
    { before: "", base: "mix(Base = Other, Base)" },
    { before: "const Alias = Base;", base: "mix(Base = Other, Alias)" },
    { before: "function change() { Base = Other; }", base: "mix(change(), Base)" },
    { before: "Base = Other;", base: "mix({}, Base)" },
  ].map(({ before, base }) => ({
    name: `mutated constructor: ${before} ${base}`,
    source: "export function mix(config, Base) { return class extends Base {}; }",
    before, base,
  })),
  ...[
    "function register(mix) { BODY }",
    "const register = (mix) => { BODY };",
    "const register = function mix() { BODY };",
    "{ BODY let mix = replacement; }",
    "function register() { BODY var mix; }",
    "try {} catch (mix) { BODY }",
    "for (const mix of replacements) { BODY }",
    "function register({ mix }) { BODY }",
    "function register(factories) { BODY }",
    "function register(WebUIElement) { BODY }",
    "function register(observable) { BODY }",
  ].map((wrapper) => ({
    name: `shadowed call site: ${wrapper}`,
    source: namedFactory,
    definition: wrapper.replace(
      "BODY",
      `customElements.define('mixin-card', class extends ${
        wrapper.includes("(factories)") ? "factories.mix" : "mix"
      }(WebUIElement, { value: '' }) { @observable own = ''; });`
    ),
  })),
  {
    name: "unshadowed registration inside function",
    source: namedFactory,
    definition: `function register() {
      customElements.define('mixin-card', class extends mix(WebUIElement, { value: '' }) {});
    }`,
    keys: ["inherited"],
  },
  {
    name: "shared factory properties do not cache the supplied base",
    source: namedFactory,
    definition: `
class First extends mix(Base, { value: '' }) {}
First.define('first-card');
class Second extends mix(Other, { value: '' }) { @observable own = ''; }
Second.define('mixin-card');`,
    keys: ["actual", "inherited", "own"],
  },
  {
    name: "cached module identity does not bypass lexical shadowing",
    source: namedFactory,
    definition: `
class First extends mix(WebUIElement, { value: '' }) {}
First.define('first-card');
function register(mix) {
  customElements.define('mixin-card', class extends mix(WebUIElement) {
    @observable own = '';
  });
}`,
  },
];

export async function assertClassFactoryCase(
  compile: (context: AdapterContext) => Promise<ProjectionManifest>,
  fixture: FactoryCase
): Promise<void> {
  const root = path.resolve(".webui-class-factory-tests");
  const id = (name: string): string => path.join(root, name);
  const framework: ResolvedImport = {
    specifier: "@microsoft/webui-framework", resolvedId: undefined, external: true,
    kind: "static", packageName: "@microsoft/webui-framework",
  };
  const edge = (specifier: string, name: string): ResolvedImport => ({
    specifier, resolvedId: id(name), external: false, kind: "static",
  });
  const modules = [
    {
      id: id("factory.ts"), kind: "file" as const,
      source: `import { WebUIElement, observable, attr } from '@microsoft/webui-framework';\n${fixture.source}`,
      imports: [framework],
    },
    {
      id: id("barrel.ts"), kind: "file" as const,
      source: "export { mix } from './factory.js';",
      imports: [edge("./factory.js", "factory.ts")],
    },
    {
      id: id("derived.ts"), kind: "file" as const,
      source: `
import { WebUIElement, observable } from '@microsoft/webui-framework';
import * as factories from './barrel.js';
const mix = factories.mix;
class Base extends WebUIElement { @observable base = ''; }
class Other extends WebUIElement { @observable actual = ''; }
${fixture.before ?? ""}
${fixture.definition ?? `
class Derived extends ${fixture.base ?? "mix(WebUIElement)"} {
  ${fixture.fields ?? "@observable own = '';"}
}
Derived.define('mixin-card');`}`,
      imports: [framework, edge("./barrel.js", "barrel.ts")],
    },
  ];
  const context: AdapterContext = {
    graph: { modules: new Map(modules.map((module) => [module.id, module])), entries: [id("derived.ts")] },
    membership: { outputs: new Map([[id("dist/index.js"), new Set(modules.map((module) => module.id))]]) },
    outputContents: new Map([[id("dist/index.js"), "compiled-output"]]),
    rootDir: root, manifestPath: id("dist/webui-projection.json"),
    bundlerName: "test", bundlerVersion: "1.0.0",
  };
  if (fixture.keys === undefined) {
    await assert.rejects(compile(context), (error: unknown) => {
      assert.ok(error instanceof Error && "diagnostics" in error, fixture.name);
      const diagnostics = error.diagnostics as ReadonlyArray<{ code: string }>;
      assert.ok(diagnostics.length > 0, fixture.name);
      assert.ok(diagnostics.every(({ code }) =>
        code === "PROJ-C005" || code === "PROJ-C004"), fixture.name);
      return true;
    });
  } else {
    const manifest = await compile(context);
    assert.deepEqual(manifest.components["mixin-card"]?.hydrationKeys, fixture.keys, fixture.name);
    assert.deepEqual(manifest.components["mixin-card"]?.navigationKeys, fixture.keys, fixture.name);
    assert.deepEqual(await compile(context), manifest, fixture.name);
  }
}
