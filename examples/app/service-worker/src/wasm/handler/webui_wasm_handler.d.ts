// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export default function init(
  module?: RequestInfo | URL | Response | BufferSource | WebAssembly.Module | {
    module_or_path?: RequestInfo | URL | Response | BufferSource | WebAssembly.Module;
  },
): Promise<unknown>;

export class Protocol {
  constructor(protocolBytes: Uint8Array, plugin?: string | null);

  streamResponse(
    entry: string,
    requestPath: string,
    options?: {
      nonce?: string;
      headInject?: string;
      bodyInject?: string;
    },
  ): StreamingSession;
}

export interface BoundaryDescriptor {
  instanceId: number;
  declarationId: number;
  owner: string;
  name: string;
  key?: string | number;
}

export interface StreamStep {
  bytes: Uint8Array;
  done: boolean;
  boundary?: BoundaryDescriptor;
}

export class StreamingSession {
  start(stateJson: string): StreamStep;
  resume(instanceId: number, stateJson: string, mode?: 'final' | 'updatable'): StreamStep;
  update(instanceId: number, stateJson: string): Uint8Array;
}
