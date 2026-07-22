// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Runs the shared adapter conformance suite against `compileProjection`
 * (the bundler-neutral TypeScript AST semantic compiler).
 *
 * This is the primary correctness oracle for the compiler: every fixture in
 * `ALL_CASES` documents an authoritative scenario from DESIGN.md's
 * "Bundler-Neutral State Projection Compiler" contract.
 */

import { describe, test } from "node:test";
import { strict as assert } from "node:assert";
import {
  runConformanceSuite,
  ALL_CASES,
  compileProjection,
} from "@microsoft/webui/projection.js";

describe("projection compiler conformance suite", () => {
  test("all fixtures pass", async () => {
    const report = await runConformanceSuite(async (ctx) => compileProjection(ctx));

    if (report.failed.length > 0) {
      const details = report.failed
        .map(
          (f) =>
            `  - ${f.id}: ${f.reason}\n    expected: ${JSON.stringify(f.expected)}\n    actual:   ${JSON.stringify(f.actual)}`
        )
        .join("\n");
      assert.fail(
        `${report.failed.length} conformance case(s) failed:\n${details}`
      );
    }

    // Every declared case must be accounted for (passed or explicitly skipped).
    assert.equal(
      report.passed.length + report.skipped.length,
      ALL_CASES.length
    );
  });

  test("fixture identifiers are unique", () => {
    const ids = ALL_CASES.map((fixture) => fixture.id);
    assert.equal(new Set(ids).size, ids.length);
  });
});
