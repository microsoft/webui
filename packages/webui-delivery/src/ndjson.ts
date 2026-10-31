// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { NdjsonReader } from './ndjson-reader.js';
import { jsonTransport, type PartialTransport } from './transport.js';

/** Select bounded NDJSON delivery, accepting a complete JSON response as well. */
export function ndjson(maxRecordBytes = 256 * 1024): PartialTransport {
  if (!Number.isSafeInteger(maxRecordBytes) || maxRecordBytes <= 0) {
    throw new RangeError('[WebUI] maxRecordBytes must be a positive safe integer.');
  }
  return {
    accept: 'application/x-ndjson, application/json',
    async read(response, signal) {
      if (!response.headers.get('content-type')?.includes('ndjson')) {
        return jsonTransport.read(response, signal);
      }
      if (!response.body) throw new Error('[WebUI] NDJSON response has no body.');
      const records = new NdjsonReader(response.body, { signal, maxRecordBytes });
      try {
        return { data: await records.read(), records };
      } catch (error) {
        await records.cancel(error);
        throw error;
      }
    },
  };
}
