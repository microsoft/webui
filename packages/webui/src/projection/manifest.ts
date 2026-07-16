// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Versioned deterministic JSON manifest schema for WebUI state projection.
 *
 * See DESIGN.md §"Bundler-Neutral State Projection Compiler" for the
 * authoritative cross-language contract.
 */

import { createHash } from "node:crypto";

/** The only valid schema string for this version. */
export const MANIFEST_SCHEMA = "webui.state-projection/v1" as const;
export type ManifestSchema = typeof MANIFEST_SCHEMA;

/** Sentinel hash value for virtual modules/outputs with no physical bytes. */
export const VIRTUAL_HASH = "virtual" as const;

/** Bytes accepted by the SHA-256 helpers. */
export type HashContent = string | Uint8Array;

/** Computes `"sha256:<64 lowercase hex>"` over exact UTF-8/byte content. */
export function hashContent(content: HashContent): string {
  const hash = createHash("sha256");
  if (typeof content === "string") hash.update(content, "utf8");
  else hash.update(content);
  return `sha256:${hash.digest("hex")}`;
}

/** Cross-language lexical ordering: raw UTF-8 bytes, ascending. */
export function compareUtf8(left: string, right: string): number {
  return Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8"));
}

/**
 * Computes the deterministic manifest `buildId`.
 *
 * Each canonical record is encoded as a record label followed by
 * `<utf8-byte-length>:<value>` fields and one LF. Length-prefixing avoids
 * delimiter ambiguity for paths and Unicode identifiers.
 */
export function computeBuildId(params: {
  readonly producerName: string;
  readonly producerVersion: string;
  readonly adapterName: string;
  readonly adapterBundler: string;
  readonly root: string;
  readonly analysisHash: string;
  readonly sortedInputs: ReadonlyArray<readonly [string, string]>;
  readonly sortedOutputs: ReadonlyArray<readonly [string, string]>;
  readonly sortedComponents: ReadonlyArray<
    readonly [
      tag: string,
      module: string,
      sortedOutputs: ReadonlyArray<string>,
      sortedHydrationKeys: ReadonlyArray<string>,
      sortedNavigationKeys: ReadonlyArray<string>,
    ]
  >;
}): string {
  const records: string[] = [];
  appendRecord(records, "schema", [MANIFEST_SCHEMA]);
  appendRecord(records, "producer", [
    params.producerName,
    params.producerVersion,
  ]);
  appendRecord(records, "adapter", [
    params.adapterName,
    params.adapterBundler,
  ]);
  appendRecord(records, "root", [params.root]);
  appendRecord(records, "analysis", [params.analysisHash]);
  appendRecord(records, "inputs", [String(params.sortedInputs.length)]);
  for (const [path, hash] of params.sortedInputs) {
    appendRecord(records, "input", [path, hash]);
  }
  appendRecord(records, "outputs", [String(params.sortedOutputs.length)]);
  for (const [path, hash] of params.sortedOutputs) {
    appendRecord(records, "output", [path, hash]);
  }
  appendRecord(records, "components", [
    String(params.sortedComponents.length),
  ]);
  for (const [
    tag,
    module,
    outputs,
    hydrationKeys,
    navigationKeys,
  ] of params.sortedComponents) {
    appendRecord(records, "component", [
      tag,
      module,
      String(outputs.length),
      ...outputs,
      String(hydrationKeys.length),
      ...hydrationKeys,
      String(navigationKeys.length),
      ...navigationKeys,
    ]);
  }
  return hashContent(records.join(""));
}

function appendRecord(
  destination: string[],
  label: string,
  fields: ReadonlyArray<string>
): void {
  let record = label;
  for (const field of fields) {
    record += `${Buffer.byteLength(field, "utf8")}:${field}`;
  }
  destination.push(`${record}\n`);
}

/** Serializes a manifest to its canonical compact JSON form. */
export function serializeManifestCanonical(
  manifest: ProjectionManifest
): string {
  const inputs = sortRecord(manifest.inputs);
  const outputs = sortRecord(manifest.outputs);
  const components: Record<string, ComponentEntry> = {};
  for (const tag of Object.keys(manifest.components).sort(compareUtf8)) {
    const entry = manifest.components[tag]!;
    components[tag] = {
      module: entry.module,
      outputs: [...entry.outputs].sort(compareUtf8),
      hydrationKeys: [...entry.hydrationKeys].sort(compareUtf8),
      navigationKeys: [...entry.navigationKeys].sort(compareUtf8),
    };
  }
  return JSON.stringify({
    schema: manifest.schema,
    producer: manifest.producer,
    adapter: manifest.adapter,
    root: manifest.root,
    analysisHash: manifest.analysisHash,
    buildId: manifest.buildId,
    outputs,
    inputs,
    components,
  });
}

function sortRecord(
  value: Readonly<Record<string, string>>
): Record<string, string> {
  const result: Record<string, string> = {};
  for (const key of Object.keys(value).sort(compareUtf8)) {
    result[key] = value[key]!;
  }
  return result;
}

/** Root manifest type. */
export interface ProjectionManifest {
  readonly schema: ManifestSchema;
  readonly producer: ProducerInfo;
  readonly adapter: AdapterInfo;

  /**
   * Build root relative to the manifest directory.
   *
   * `"."` or one/more parent segments (`".."`, `"../.."`). Every physical
   * input/output/module path is relative to this root and contains no `..`.
   */
  readonly root: string;

  /** SHA-256 of normalized entries, graph edges, and output membership. */
  readonly analysisHash: string;

  /** Deterministic identifier covering the complete serialized proof. */
  readonly buildId: string;

  /** Root-relative output path to content hash. */
  readonly outputs: Readonly<Record<string, string>>;

  /** Root-relative/virtual input ID to content hash. */
  readonly inputs: Readonly<Record<string, string>>;

  /** Exact component surfaces keyed by custom-element tag. */
  readonly components: Readonly<Record<string, ComponentEntry>>;
}

/** Identity of the tool that produced the manifest. */
export interface ProducerInfo {
  readonly name: "@microsoft/webui/projection.js";
  readonly version: string;
}

/** Identity of the bundler adapter. */
export interface AdapterInfo {
  readonly name: string;
  readonly bundler: string;
}

/** One shipped WebUI component's exact client state surface. */
export interface ComponentEntry {
  /** Root-relative physical module path. */
  readonly module: string;
  /** Sorted root-relative/virtual outputs containing the defining module. */
  readonly outputs: ReadonlyArray<string>;
  /**
   * Sorted exact `@observable + @attr` property names consumed during initial
   * SSR bootstrap. Existing host attributes take precedence; state fills
   * attribute-backed values that were not materialized on the SSR host.
   */
  readonly hydrationKeys: ReadonlyArray<string>;

  /**
   * Sorted exact property names consumed by client-created/navigation state:
   * `@observable` plus `@attr`. Attribute decorators still use the JavaScript
   * property name because `setState()` addresses properties, not reflected
   * HTML attribute names.
   */
  readonly navigationKeys: ReadonlyArray<string>;
}

/**
 * Validates a parsed manifest's structural and canonical invariants.
 *
 * Stale file validation is intentionally performed by the consuming host.
 */
export function validateManifestSchema(value: unknown): string[] {
  const errors = new Set<string>();
  if (!isRecord(value)) return ["PROJ-M008"];
  rejectUnknownKeys(
    value,
    [
      "schema",
      "producer",
      "adapter",
      "root",
      "analysisHash",
      "buildId",
      "outputs",
      "inputs",
      "components",
    ],
    errors
  );

  if (value["schema"] !== MANIFEST_SCHEMA) errors.add("PROJ-M002");
  if (!isProducerInfo(value["producer"])) errors.add("PROJ-M009");
  if (!isAdapterInfo(value["adapter"])) errors.add("PROJ-M009");
  if (
    typeof value["root"] !== "string" ||
    !isCanonicalRoot(value["root"])
  ) {
    errors.add("PROJ-S003");
  }
  if (!isSha256(value["analysisHash"]) || !isSha256(value["buildId"])) {
    errors.add("PROJ-S004");
  }

  const inputs = validateHashRecord(value["inputs"], errors);
  const outputs = validateHashRecord(value["outputs"], errors);
  validateComponents(value["components"], inputs, outputs, errors);
  return [...errors];
}

function validateHashRecord(
  value: unknown,
  errors: Set<string>
): Readonly<Record<string, string>> | undefined {
  if (!isRecord(value)) {
    errors.add("PROJ-M009");
    return undefined;
  }
  const keys = Object.keys(value);
  if (!isSortedUnique(keys)) errors.add("PROJ-M009");
  for (const [path, hash] of Object.entries(value)) {
    if (typeof hash !== "string" || !isHash(hash)) {
      errors.add("PROJ-S004");
      continue;
    }
    const virtual = isVirtualPath(path);
    if (!virtual && !isCanonicalFilePath(path)) errors.add("PROJ-S003");
    if (
      (virtual && hash !== VIRTUAL_HASH) ||
      (!virtual && hash === VIRTUAL_HASH)
    ) {
      errors.add("PROJ-S004");
    }
  }
  return value as Readonly<Record<string, string>>;
}

function validateComponents(
  value: unknown,
  inputs: Readonly<Record<string, string>> | undefined,
  outputs: Readonly<Record<string, string>> | undefined,
  errors: Set<string>
): void {
  if (!isRecord(value)) {
    errors.add("PROJ-M009");
    return;
  }
  const tags = Object.keys(value);
  if (tags.length > 65_535) errors.add("PROJ-S002");
  if (!isSortedUnique(tags)) errors.add("PROJ-M009");

  for (const [tag, rawEntry] of Object.entries(value)) {
    if (!isComponentTag(tag) || !isRecord(rawEntry)) {
      errors.add("PROJ-M009");
      continue;
    }
    rejectUnknownKeys(
      rawEntry,
      ["module", "outputs", "hydrationKeys", "navigationKeys"],
      errors
    );
    const module = rawEntry["module"];
    const componentOutputs = rawEntry["outputs"];
    const hydrationKeys = rawEntry["hydrationKeys"];
    const navigationKeys = rawEntry["navigationKeys"];
    if (
      typeof module !== "string" ||
      !isCanonicalFilePath(module) ||
      inputs?.[module] === undefined ||
      inputs[module] === VIRTUAL_HASH
    ) {
      errors.add("PROJ-M009");
    }
    if (
      !isStringArray(componentOutputs) ||
      componentOutputs.length === 0 ||
      !isSortedUnique(componentOutputs)
    ) {
      errors.add("PROJ-M009");
    } else {
      for (const output of componentOutputs) {
        if (outputs?.[output] === undefined) errors.add("PROJ-M009");
      }
    }
    if (
      !isStringArray(hydrationKeys) ||
      !isSortedUnique(hydrationKeys) ||
      !isStringArray(navigationKeys) ||
      !isSortedUnique(navigationKeys)
    ) {
      errors.add("PROJ-M009");
    } else if (
      hydrationKeys.some(
        (key) => key.length === 0 || hasControlCharacter(key)
      ) ||
      navigationKeys.some(
        (key) => key.length === 0 || hasControlCharacter(key)
      ) ||
      hydrationKeys.some(
        (key) => binarySearch(navigationKeys, key) === false
      )
    ) {
      errors.add("PROJ-M009");
    }
  }
}

function rejectUnknownKeys(
  value: Readonly<Record<string, unknown>>,
  allowed: ReadonlyArray<string>,
  errors: Set<string>
): void {
  const allowedSet = new Set(allowed);
  if (Object.keys(value).some((key) => !allowedSet.has(key))) {
    errors.add("PROJ-M009");
  }
}

function isProducerInfo(value: unknown): boolean {
  if (!isRecord(value)) return false;
  const keys = Object.keys(value);
  return (
    keys.length === 2 &&
    keys.includes("name") &&
    keys.includes("version") &&
    value["name"] === "@microsoft/webui/projection.js" &&
    typeof value["version"] === "string" &&
    value["version"].length > 0
  );
}

function isAdapterInfo(value: unknown): boolean {
  if (!isRecord(value)) return false;
  const keys = Object.keys(value);
  return (
    keys.length === 2 &&
    keys.includes("name") &&
    keys.includes("bundler") &&
    typeof value["name"] === "string" &&
    value["name"].length > 0 &&
    typeof value["bundler"] === "string" &&
    value["bundler"].length > 0
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((entry) => typeof entry === "string");
}

function isHash(value: string): boolean {
  return value === VIRTUAL_HASH || isSha256(value);
}

function isSha256(value: unknown): value is string {
  if (typeof value !== "string" || value.length !== 71) return false;
  if (!value.startsWith("sha256:")) return false;
  for (let index = 7; index < value.length; index++) {
    const code = value.charCodeAt(index);
    const digit = code >= 48 && code <= 57;
    const lowerHex = code >= 97 && code <= 102;
    if (!digit && !lowerHex) return false;
  }
  return true;
}

function isCanonicalRoot(value: string): boolean {
  if (value === ".") return true;
  const segments = value.split("/");
  return segments.length <= 32 && segments.every((segment) => segment === "..");
}

function isVirtualPath(value: string): boolean {
  return value.startsWith("virtual:");
}

function isCanonicalFilePath(value: string): boolean {
  if (
    value.length === 0 ||
    value.startsWith("/") ||
    value.startsWith("./") ||
    value.includes("\\") ||
    hasControlCharacter(value)
  ) {
    return false;
  }
  const segments = value.split("/");
  return segments.every(
    (segment) => segment.length > 0 && segment !== "." && segment !== ".."
  );
}

function isComponentTag(value: string): boolean {
  if (value.length < 3 || !value.includes("-")) return false;
  for (let index = 0; index < value.length; index++) {
    const code = value.charCodeAt(index);
    const valid =
      (code >= 97 && code <= 122) ||
      (code >= 48 && code <= 57) ||
      code === 45 ||
      code === 46 ||
      code === 95;
    if (!valid) return false;
  }
  return value.charCodeAt(0) >= 97 && value.charCodeAt(0) <= 122;
}

function isSortedUnique(values: ReadonlyArray<string>): boolean {
  for (let index = 1; index < values.length; index++) {
    if (compareUtf8(values[index - 1]!, values[index]!) >= 0) return false;
  }
  return true;
}

function binarySearch(
  values: ReadonlyArray<string>,
  target: string
): boolean {
  let low = 0;
  let high = values.length;
  while (low < high) {
    const middle = low + ((high - low) >> 1);
    const order = compareUtf8(values[middle]!, target);
    if (order === 0) return true;
    if (order < 0) low = middle + 1;
    else high = middle;
  }
  return false;
}

function hasControlCharacter(value: string): boolean {
  for (let index = 0; index < value.length; index++) {
    const code = value.charCodeAt(index);
    if (code <= 31 || code === 127) return true;
  }
  return false;
}
