// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Bounded records owned by one response, independent of its UI consumer. */
export interface PartialRecords {
  readBuffered(): unknown | undefined;
  drain(consume: (value: unknown) => void): Promise<void>;
  cancel(reason?: unknown): Promise<void>;
}

/** Initial response data and an optional unread record tail. */
export interface PartialBody {
  readonly data: unknown;
  readonly records?: PartialRecords;
}

/** Selected response format for route partials. */
export interface PartialTransport {
  readonly accept: string;
  read(response: Response, signal: AbortSignal): Promise<PartialBody>;
}

/** JSON delivery does not import a stream decoder. */
export const jsonTransport: PartialTransport = {
  accept: 'application/json',
  async read(response, signal) {
    const type = response.headers.get('content-type') ?? '';
    if (type.includes('ndjson')) {
      await response.body?.cancel();
      throw new Error('[WebUI] Received NDJSON for JSON-only delivery. Select ndjson() or return JSON.');
    }
    const data: unknown = await response.json();
    signal.throwIfAborted();
    return { data };
  },
};
