// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { Hello } from './types.js';

/** Local descriptors and other own properties are not native control fields. */
export function projectHello(value: Hello): Hello {
  return {
    wireVersion: value.wireVersion,
    contractName: value.contractName,
    contractMajor: value.contractMajor,
    schemaHash: value.schemaHash,
  };
}
