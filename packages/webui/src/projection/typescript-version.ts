// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export const TYPESCRIPT_PEER_RANGE = "^6.0.3 || 7.0.2";

export type SupportedTypeScriptMajor = 6 | 7;

export function supportedTypeScriptMajor(
  version: string
): SupportedTypeScriptMajor | undefined {
  const parts = parseVersion(version);
  if (parts === undefined) return undefined;
  if (
    parts.major === 6 &&
    (parts.minor > 0 ||
      (parts.minor === 0 && parts.patch >= 3))
  ) {
    return 6;
  }
  if (
    parts.major === 7 &&
    parts.minor === 0 &&
    parts.patch === 2
  ) {
    return 7;
  }
  return undefined;
}

function parseVersion(
  value: string
): { major: number; minor: number; patch: number } | undefined {
  const buildOffset = value.indexOf("+");
  if (
    buildOffset !== -1 &&
    !isValidBuildMetadata(value.slice(buildOffset + 1))
  ) {
    return undefined;
  }
  const core = buildOffset === -1 ? value : value.slice(0, buildOffset);
  if (core.includes("-")) return undefined;
  const parts = core.split(".");
  if (parts.length !== 3) return undefined;
  const major = parseNumericSegment(parts[0]);
  const minor = parseNumericSegment(parts[1]);
  const patch = parseNumericSegment(parts[2]);
  return major !== undefined &&
    minor !== undefined &&
    patch !== undefined
    ? { major, minor, patch }
    : undefined;
}

function parseNumericSegment(value: string | undefined): number | undefined {
  if (value === undefined || value.length === 0) return undefined;
  if (value.length > 1 && value.charCodeAt(0) === 48) return undefined;
  for (let index = 0; index < value.length; index++) {
    const code = value.charCodeAt(index);
    if (code < 48 || code > 57) return undefined;
  }
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : undefined;
}

function isValidBuildMetadata(value: string): boolean {
  if (value.length === 0) return false;
  let segmentLength = 0;
  for (let index = 0; index < value.length; index++) {
    const code = value.charCodeAt(index);
    if (code === 46) {
      if (segmentLength === 0) return false;
      segmentLength = 0;
      continue;
    }
    const valid =
      (code >= 48 && code <= 57) ||
      (code >= 65 && code <= 90) ||
      code === 45 ||
      (code >= 97 && code <= 122);
    if (!valid) return false;
    segmentLength++;
  }
  return segmentLength > 0;
}
