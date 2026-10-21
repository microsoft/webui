// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { NdjsonReader } from './ndjson-reader.js';
import { lazyRecord, type LazyTransport } from './lazy-data.js';
import { abortable } from './abortable.js';

/** Select bounded two-phase lazy delivery without loading navigation code. */
export function createLazyNdjson(maxRecordBytes = 256 * 1024): LazyTransport {
  if (!Number.isSafeInteger(maxRecordBytes) || maxRecordBytes <= 0) {
    throw new RangeError('[WebUI] Lazy maxRecordBytes must be a positive safe integer.');
  }
  return {
    accept: 'application/x-ndjson, application/json',
    async read(response, context) {
      const contentType = response.headers.get('content-type') ?? '';
      if (!contentType.includes('ndjson')) {
        const record = lazyRecord(await response.json(), context);
        if (record.state === undefined) throw new Error('[WebUI] Lazy response ended without state.');
        await abortable(context.prepare(record), context.signal);
        context.signal.throwIfAborted();
        return record.state;
      }
      if (!response.body) throw new Error('[WebUI] Lazy response has no body.');
      const reader = new NdjsonReader(response.body, { signal: context.signal, maxRecordBytes });
      const preparations: Promise<void>[] = [];
      let state: Record<string, unknown> | undefined;
      let preparationError: unknown;
      let count = 0;
      try {
        while (true) {
          const value = await reader.read();
          if (value === undefined) break;
          if (++count > 2 || state !== undefined) {
            throw new Error('[WebUI] Lazy responses must contain resources followed by one terminal state record.');
          }
          const record = lazyRecord(value, context);
          const preparing = context.prepare(record);
          preparations.push(preparing);
          void preparing.catch(error => {
            preparationError ??= error;
            context.abort(error);
          });
          if (record.state !== undefined) state = record.state;
        }
        if (state === undefined) throw new Error('[WebUI] Lazy response ended before its state record.');
        await abortable(Promise.all(preparations), context.signal);
        context.signal.throwIfAborted();
        return state;
      } catch (error) {
        throw preparationError ?? error;
      } finally {
        await reader.cancel();
      }
    },
  };
}

/** Default 256-KiB-per-record lazy transport. */
export const lazyNdjson = createLazyNdjson();
