// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { mkdir, open, rename, rm } from "node:fs/promises";
import * as path from "node:path";

let sequence = 0;

/** Publish one build artifact without exposing a partially written manifest. */
export async function writeAtomic(filename: string, contents: string): Promise<void> {
  await mkdir(path.dirname(filename), { recursive: true });
  const temporary = `${filename}.tmp-${process.pid}-${sequence++}`;
  let created = false;
  try {
    const handle = await open(temporary, "wx");
    created = true;
    try {
      await handle.writeFile(contents, "utf8");
      await handle.sync();
    } finally {
      await handle.close();
    }
    await rename(temporary, filename);
  } finally {
    if (created) await rm(temporary, { force: true });
  }
}
