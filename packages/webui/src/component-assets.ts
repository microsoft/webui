// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import fs from "node:fs";
import nodePath from "node:path";

interface OrderedOutput {
  name: string;
  kind: 0 | 1;
  orderKey: string;
}

/** Read component asset outputs in the same root-then-chunk order as the native API. */
export function readComponentAssetFiles(
  outDir: string,
  metafileJson: string | undefined,
): string[] {
  if (!metafileJson) return [];
  const metafile: unknown = JSON.parse(metafileJson);
  if (!isRecord(metafile) || !isRecord(metafile.outputs)) {
    throw new Error(
      "[webui] Component asset metafile must contain an outputs object.",
    );
  }

  const ordered = orderOutputs(metafile.outputs);
  const files: string[] = [];
  for (let i = 0; i < ordered.length; i++) {
    const name = ordered[i].name;
    if (nodePath.basename(name) !== name) {
      throw new Error(
        `[webui] Component asset metafile output must be a filename: ${name}`,
      );
    }
    const path = nodePath.join(outDir, name);
    files.push(name, fs.readFileSync(path, "utf8"));
  }
  return files;
}

function orderOutputs(outputs: Record<string, unknown>): OrderedOutput[] {
  const names = Object.keys(outputs);
  const ordered = new Array<OrderedOutput>(names.length);
  for (let i = 0; i < names.length; i++) {
    const name = names[i];
    const output = outputs[name];
    if (!isRecord(output) || !isRecord(output.inputs)) {
      throw new Error(
        `[webui] Component asset metafile output ${name} must contain an inputs object.`,
      );
    }
    if (output.entryPoint !== undefined && typeof output.entryPoint !== "string") {
      throw new Error(
        `[webui] Component asset metafile output ${name} has an invalid entryPoint.`,
      );
    }

    if (typeof output.entryPoint === "string") {
      ordered[i] = { name, kind: 0, orderKey: output.entryPoint };
      continue;
    }

    const inputs = Object.keys(output.inputs).sort();
    if (inputs.length === 0) {
      throw new Error(
        `[webui] Shared component asset metafile output ${name} has no component input.`,
      );
    }
    ordered[i] = { name, kind: 1, orderKey: inputs[0] };
  }

  ordered.sort((left, right) => {
    if (left.kind !== right.kind) return left.kind - right.kind;
    if (left.orderKey < right.orderKey) return -1;
    if (left.orderKey > right.orderKey) return 1;
    if (left.name < right.name) return -1;
    if (left.name > right.name) return 1;
    return 0;
  });
  return ordered;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
