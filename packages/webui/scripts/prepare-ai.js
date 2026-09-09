// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { copyFile } from "node:fs/promises";

await copyFile(
  new URL("../../../docs/ai.md", import.meta.url),
  new URL("../ai.md", import.meta.url),
);
