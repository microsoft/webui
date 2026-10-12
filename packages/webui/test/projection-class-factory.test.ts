// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { describe, test } from "node:test";
import { compileProjection } from "@microsoft/webui/projection.js";
import { assertClassFactoryCase, CLASS_FACTORY_CASES } from "./projection-class-factory-cases.js";

describe("class factory projection", () => {
  for (const fixture of CLASS_FACTORY_CASES) {
    test(fixture.name, () => assertClassFactoryCase(compileProjection, fixture));
  }
});
