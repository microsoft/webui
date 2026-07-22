// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export default function init(
  module?: RequestInfo | URL | Response | BufferSource | WebAssembly.Module | {
    module_or_path?: RequestInfo | URL | Response | BufferSource | WebAssembly.Module;
  },
): Promise<unknown>;

export class Protocol {
  constructor(protocolBytes: Uint8Array, plugin?: string | null);

  renderStream(
    stateJson: string,
    onChunk: (html: string) => void,
    options?: {
      entry?: string;
      requestPath?: string;
    },
  ): void;
}
