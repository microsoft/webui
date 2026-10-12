// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import * as path from "node:path";
import { test } from "node:test";
import { build, Protocol } from "@microsoft/webui";
import { esbuildProjection } from "@microsoft/webui/projection.js";
import type { ProjectionManifest } from "@microsoft/webui/projection.js";
import * as esbuild from "esbuild";

const MODELS = `
import { WebUIElement, observable } from '@microsoft/webui-framework';
export class First extends WebUIElement { @observable claimed = ''; }
export class Other extends WebUIElement { @observable actual = ''; }
export function mix(config, Base) {
  class Mixed extends Base { @observable own = ''; }
  return Mixed;
}
`;

const FACTORY = `
import { WebUIElement, observable } from '@microsoft/webui-framework';
function mix(Base) {
  class Mixed extends Base { @observable inherited = ''; }
  return Mixed;
}
`;

const CASES = [
  {
    name: "rejects constructor assignment in another argument",
    source: `
import { First, Other, mix } from './models.js';
class Base extends First {}
class Derived extends mix(Base = Other, Base) {}
Derived.define('probe-card');`,
    error: /PROJ-C005/,
  },
  {
    name: "rejects a mutated constructor reached through an alias",
    source: `
import { First, Other, mix } from './models.js';
class Base extends First {}
const Alias = Base;
class Derived extends mix(Base = Other, Alias) {}
Derived.define('probe-card');`,
    error: /PROJ-C005/,
  },
  {
    name: "rejects a factory shadowed by a registration parameter",
    source: `${FACTORY}
function other(Base) {
  class Other extends Base { @observable actual = ''; }
  return Other;
}
function register(mix) {
  customElements.define('probe-card', class extends mix(WebUIElement) {});
}
register(other);`,
    error: /PROJ-B001/,
  },
  ...[
    "function replace(value = (mix = other)) { var mix; } replace();",
    "class Effects { [(mix = other, 'method')](mix) {} }",
    "(mix as any) = other;",
    "mix! = other;",
  ].map((statement) => ({
    name: `rejects module mutation in ${statement}`,
    source: `${FACTORY}
function other(Base) {
  class Other extends Base { @observable actual = ''; }
  return Other;
}
${statement}
class Derived extends mix(WebUIElement) { @observable own = ''; }
Derived.define('probe-card');`,
    error: /PROJ-C005/,
  })),
  ...[
    "for (const { factory = mix } of [{}]) { void factory; }",
    "for (const { mix: selected } of [{ mix }]) { void selected; }",
    "function unrelated(mix) { mix = null; }",
    "class Holder { static { var mix; mix = null; } }",
  ].map((statement) => ({
    name: `preserves precise state with ${statement}`,
    source: `${FACTORY}
${statement}
class Derived extends mix(WebUIElement) { @observable own = ''; }
Derived.define('probe-card');`,
    error: undefined,
  })),
];

for (const fixture of CASES) {
  test(fixture.name, async (t) => {
    const root = await mkdtemp(path.join(process.cwd(), ".tmp-esbuild-projection-proof-"));
    t.after(() => rm(root, { recursive: true, force: true }));
    const appDir = path.join(root, "src");
    await mkdir(appDir);
    await Promise.all([
      writeFile(path.join(root, "package.json"), '{"name":"projection-proof-fixture","type":"module"}'),
      writeFile(path.join(appDir, "models.ts"), MODELS),
      writeFile(path.join(appDir, "probe-card.ts"), fixture.source),
      writeFile(path.join(appDir, "probe-card.html"), "<p>{{serverNote}}</p>"),
      writeFile(path.join(appDir, "index.html"),
        '<!doctype html><html><head></head><body><probe-card></probe-card></body></html>'),
    ]);
    const manifestPath = path.join(root, "dist", "webui-projection.json");
    const compile = async (): Promise<Protocol> => {
      await esbuild.build({
        absWorkingDir: root,
        entryPoints: ["src/probe-card.ts"],
        outdir: "dist",
        bundle: true,
        format: "esm",
        logLevel: "silent",
        tsconfigRaw: { compilerOptions: {
          experimentalDecorators: true,
          useDefineForClassFields: false,
        } },
        alias: { "@microsoft/webui-framework": path.resolve("../webui-framework/src/index.ts") },
        plugins: [esbuildProjection()],
      });
      return new Protocol(build({
        appDir, plugin: "webui", projectionManifests: [manifestPath],
      }).protocol, { plugin: "webui" });
    };
    if (fixture.error) {
      await assert.rejects(compile(), fixture.error);
      return;
    }

    const protocol = await compile();
    const manifest = JSON.parse(await readFile(manifestPath, "utf8")) as ProjectionManifest;
    assert.deepEqual(manifest.components["probe-card"]?.hydrationKeys, ["inherited", "own"]);
    assert.deepEqual(manifest.components["probe-card"]?.navigationKeys, ["inherited", "own"]);
    const state = {
      inherited: "Inherited value", own: "Own value",
      serverNote: "Still rendered", unusedSentinel: "Not sent",
    };
    const html = protocol.render(state).toString("utf8");
    assert.ok(html.includes(state.serverNote));
    const marker = html.indexOf('id="webui-data"');
    assert.notEqual(marker, -1);
    const start = html.indexOf(">", marker) + 1;
    const end = html.indexOf("</script>", start);
    const bootstrap = JSON.parse(html.slice(start, end)) as { state: Record<string, unknown> };
    assert.deepEqual(bootstrap.state, { inherited: state.inherited, own: state.own });
    const partial = JSON.parse(protocol.renderPartial(state, "index.html", "/", "")) as {
      state: Record<string, unknown>;
    };
    assert.deepEqual(partial.state, {
      inherited: state.inherited, own: state.own, serverNote: state.serverNote,
    });
  });
}
