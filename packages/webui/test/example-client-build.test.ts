// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from "node:assert";
import { execFile } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import * as path from "node:path";
import { test, type TestContext } from "node:test";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";

const execute = promisify(execFile);
const helperUrl = pathToFileURL(
  path.resolve("../../examples/build-client.mjs")
).href;

async function runExampleBuild(
  t: TestContext,
  define: Record<string, string> | undefined,
  watch: boolean
): Promise<unknown> {
  const root = await mkdtemp(path.join(process.cwd(), ".tmp-example-client-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const esbuild = path.join(root, "node_modules/esbuild");
  const projection = path.join(root, "node_modules/@microsoft/webui");
  await mkdir(esbuild, { recursive: true });
  await mkdir(projection, { recursive: true });
  await writeFile(path.join(root, "package.json"), '{"type":"module"}');
  await writeFile(
    path.join(esbuild, "package.json"),
    '{"name":"esbuild","main":"index.cjs"}'
  );
  await writeFile(path.join(esbuild, "index.cjs"), `
exports.calls = [];
function record(method, options) {
  exports.calls.push({
    method,
    define: options.define ?? null,
    minify: options.minify,
    sourcemap: options.sourcemap,
  });
}
exports.build = async (options) => record("build", options);
exports.context = async (options) => {
  record("context", options);
  return { watch: async () => { exports.calls.push({ method: "watch" }); } };
};
`);
  await writeFile(
    path.join(projection, "package.json"),
    JSON.stringify({
      name: "@microsoft/webui",
      type: "module",
      exports: {
        "./projection.js": { import: { default: "./projection.js" } },
      },
    })
  );
  await writeFile(
    path.join(projection, "projection.js"),
    "export function esbuildProjection() { return { name: 'projection' }; }\n"
  );
  const script = `
import { createRequire } from "node:module";
import path from "node:path";
const options = JSON.parse(process.argv[1]);
const before = JSON.stringify(options);
process.argv[1] = "example-client-test.mjs";
const { runWebUIClientBuild } = await import(${JSON.stringify(helperUrl)});
await runWebUIClientBuild(options);
const require = createRequire(path.resolve("package.json"));
process.stdout.write(JSON.stringify({
  calls: require("esbuild").calls,
  optionsUnchanged: JSON.stringify(options) === before,
}));
`;
  const { stdout } = await execute(
    process.execPath,
    [
      "--input-type=module", "--eval", script, "--",
      JSON.stringify({ define }), ...(watch ? ["--watch"] : []),
    ],
    { cwd: root }
  );
  return JSON.parse(stdout);
}

const cases: Array<{
  name: string;
  watch: boolean;
  define: Record<string, string> | undefined;
  expected: Record<string, string> | null;
}> = [
  {
    name: "production builds disable development diagnostics",
    watch: false,
    define: undefined,
    expected: { __WEBUI_DEV__: "false" },
  },
  {
    name: "custom definitions retain the production diagnostic default",
    watch: false,
    define: { APP_FEATURE: "123" },
    expected: { __WEBUI_DEV__: "false", APP_FEATURE: "123" },
  },
  {
    name: "explicit diagnostic definitions override the production default",
    watch: false,
    define: { __WEBUI_DEV__: "true", APP_FEATURE: "123" },
    expected: { __WEBUI_DEV__: "true", APP_FEATURE: "123" },
  },
  {
    name: "watch builds preserve undefined development diagnostics",
    watch: true,
    define: undefined,
    expected: null,
  },
  {
    name: "watch builds preserve explicit definitions",
    watch: true,
    define: { __WEBUI_DEV__: "false", APP_FEATURE: "123" },
    expected: { __WEBUI_DEV__: "false", APP_FEATURE: "123" },
  },
];

for (const entry of cases) {
  test(`example client build: ${entry.name}`, async (t) => {
    assert.deepEqual(await runExampleBuild(t, entry.define, entry.watch), {
      calls: [
        {
          method: entry.watch ? "context" : "build",
          define: entry.expected,
          minify: !entry.watch,
          sourcemap: entry.watch,
        },
        ...(entry.watch ? [{ method: "watch" }] : []),
      ],
      optionsUnchanged: true,
    });
  });
}
