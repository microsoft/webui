// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Keep this build-only name resolution aligned with the framework's toKebabCase.
const CONCATENATED_ATTRIBUTES = new Set([
  "accessKey",
  "autoCapitalize",
  "contentEditable",
  "crossOrigin",
  "dirName",
  "fetchPriority",
  "formAction",
  "formEnctype",
  "formMethod",
  "formNoValidate",
  "formTarget",
  "inputMode",
  "isMap",
  "maxLength",
  "minLength",
  "noModule",
  "noValidate",
  "readOnly",
  "referrerPolicy",
  "tabIndex",
  "useMap",
]);

/** Resolves a default framework attribute name without evaluating component code. */
export function defaultAttributeName(property: string): string {
  if (CONCATENATED_ATTRIBUTES.has(property)) return property.toLowerCase();
  const firstSuffixCode = property.charCodeAt(4);
  if (
    property.startsWith("aria") &&
    firstSuffixCode >= 65 &&
    firstSuffixCode <= 90
  ) {
    return "aria-" + property.slice(4).toLowerCase();
  }
  let name = "";
  for (let index = 0; index < property.length; index++) {
    const code = property.charCodeAt(index);
    name += code >= 65 && code <= 90
      ? "-" + String.fromCharCode(code + 32)
      : property[index];
  }
  return name;
}

/** Whether a name can be represented by the projection HTML attribute registry. */
export function isValidAttributeName(name: string): boolean {
  if (name.length === 0) return false;
  for (let index = 0; index < name.length; index++) {
    const code = name.charCodeAt(index);
    if (
      code <= 32 || code === 34 || code === 39 || code === 60 ||
      code === 62 || code === 47 || code === 61 || code === 96
    ) {
      return false;
    }
  }
  return true;
}
