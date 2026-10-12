// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import * as path from "node:path";
import { describe, test } from "node:test";
import { build, Protocol } from "@microsoft/webui";
import type { PartialResponse } from "@microsoft/webui";
import { esbuildProjection } from "@microsoft/webui/projection.js";
import type { ProjectionManifest } from "@microsoft/webui/projection.js";
import * as esbuild from "esbuild";

const FRAMEWORK_ENTRY = path.resolve("../webui-framework/src/index.ts");

const MIXINS = `
import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

type ElementConstructor = new (...args: any[]) => WebUIElement;

export function withCounter<TBase extends ElementConstructor>(Base: TBase) {
  class Counter extends Base {
    @observable inheritedValue = 0;
    connectionCount = 0;

    connectedCallback(): void {
      super.connectedCallback();
      this.connectionCount += 1;
    }

    incrementInherited(): void {
      this.inheritedValue += 1;
    }
  }
  return Counter;
}

export function withCaption<TBase extends ElementConstructor>(
  Base: TBase,
  configuration: { caption: string }
) {
  class Caption extends Base {
    @attr inheritedLabel = configuration.caption;
    disconnectionCount = 0;

    disconnectedCallback(): void {
      super.disconnectedCallback();
      this.disconnectionCount += 1;
    }
  }
  return Caption;
}
`;

const OWN_STATE = { label: "Own label", ownValue: 7 };
const CASES = [
  {
    name: "direct inheritance control",
    base: "WebUIElement",
    imports: "",
    clientState: OWN_STATE,
    inheritedTemplate: "",
  },
  {
    name: "single imported factory",
    base: "withCounter(WebUIElement)",
    imports: "import { withCounter } from './mixins.js';",
    clientState: { ...OWN_STATE, inheritedValue: 11 },
    inheritedTemplate: "<span>{{inheritedValue}}</span>",
  },
  {
    name: "nested imported factories with configuration",
    base: "withCaption(withCounter(WebUIElement), { caption: 'Default caption' })",
    imports: "import { withCaption, withCounter } from './mixins.js';",
    clientState: {
      ...OWN_STATE,
      inheritedValue: 11,
      inheritedLabel: "Inherited label",
    },
    inheritedTemplate:
      "<span>{{inheritedValue}}</span><span>{{inheritedLabel}}</span>",
  },
];

describe("mixin factory projection", () => {
  for (const fixture of CASES) {
    test(fixture.name, async (t) => {
      const root = await mkdtemp(
        path.join(process.cwd(), ".tmp-esbuild-projection-mixins-")
      );
      t.after(() => rm(root, { recursive: true, force: true }));
      const appDir = path.join(root, "src");
      await mkdir(appDir);
      await Promise.all([
        writeFile(
          path.join(root, "package.json"),
          JSON.stringify({ name: "projection-mixins-fixture", type: "module" })
        ),
        writeFile(path.join(appDir, "mixins.ts"), MIXINS),
        writeFile(
          path.join(appDir, "probe-mixin.ts"),
          `
import { WebUIElement, attr, observable } from '@microsoft/webui-framework';
${fixture.imports}
class ProbeMixin extends ${fixture.base} {
  @observable ownValue = 0;
  @attr label = '';

  increment(): void {
    this.ownValue += 1;
  }
}
ProbeMixin.define('probe-mixin');
`
        ),
        writeFile(
          path.join(appDir, "probe-mixin.html"),
          `<p>{{label}}</p>
<button @click="{increment()}">{{ownValue}}</button>
${fixture.inheritedTemplate}
<p class="ssr-only">{{serverNote}}</p>`
        ),
        writeFile(
          path.join(appDir, "index.html"),
          `<!doctype html><html lang="en"><head><meta charset="utf-8"></head>
<body><probe-mixin></probe-mixin>
<script type="module" src="./probe-mixin.js"></script></body></html>`
        ),
      ]);

      await esbuild.build({
        absWorkingDir: root,
        entryPoints: ["src/probe-mixin.ts"],
        outdir: "dist",
        bundle: true,
        format: "esm",
        write: true,
        tsconfigRaw: {
          compilerOptions: {
            experimentalDecorators: true,
            useDefineForClassFields: false,
          },
        },
        alias: { "@microsoft/webui-framework": FRAMEWORK_ENTRY },
        plugins: [esbuildProjection()],
      });

      const manifestPath = path.join(root, "dist", "webui-projection.json");
      const manifest = JSON.parse(
        await readFile(manifestPath, "utf8")
      ) as ProjectionManifest;
      const expectedKeys = Object.keys(fixture.clientState).sort();
      assert.deepEqual(Object.keys(manifest.components), ["probe-mixin"]);
      assert.deepEqual(
        manifest.components["probe-mixin"]?.hydrationKeys,
        expectedKeys
      );
      assert.deepEqual(
        manifest.components["probe-mixin"]?.navigationKeys,
        expectedKeys
      );

      const result = build({
        appDir,
        plugin: "webui",
        projectionManifests: [manifestPath],
      });
      const protocol = new Protocol(result.protocol, { plugin: "webui" });
      const serverNote = "Rendered on the server only";
      const state = {
        ...fixture.clientState,
        serverNote,
        unusedSentinel: "Must not be serialized",
      };
      const html = protocol.render(state).toString("utf8");
      assert.ok(html.includes(serverNote), "SSR-only content must still render");
      assert.ok(html.includes("Own label"));
      if ("inheritedLabel" in fixture.clientState) {
        assert.ok(html.includes("Inherited label"));
      }
      const dataMarker = html.indexOf('id="webui-data"');
      assert.notEqual(dataMarker, -1, "hydration bootstrap must be present");
      const dataStart = html.indexOf(">", dataMarker) + 1;
      const dataEnd = html.indexOf("</script>", dataStart);
      assert.notEqual(dataEnd, -1);
      const bootstrap = JSON.parse(html.slice(dataStart, dataEnd)) as {
        state: Record<string, unknown>;
      };
      assert.deepEqual(bootstrap.state, fixture.clientState);
      assert.ok(!html.includes(state.unusedSentinel));

      const partial = JSON.parse(
        protocol.renderPartial(state, "index.html", "/", "")
      ) as PartialResponse;
      assert.deepEqual(partial.state, { ...fixture.clientState, serverNote });
    });
  }
});
