// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { ServerResponse } from 'node:http';

export const WEBUI_STREAM_MEDIA_TYPE = 'application/x-webui-stream';

const WEBUI_STREAM_VERSION = 1;
const MAX_RECORD_BYTES = 2_000_000;

export type JsonValue =
  | boolean
  | number
  | string
  | null
  | JsonObject
  | readonly JsonValue[];

export interface JsonObject {
  readonly [key: string]: JsonValue;
}

type StreamRecord =
  | { type: 'shell'; version: typeof WEBUI_STREAM_VERSION; state: JsonObject }
  | { type: 'boundary'; name: string; mode?: 'updatable'; state?: JsonObject }
  | { type: 'update'; name: string; state: JsonObject }
  | { type: 'finish'; state?: JsonObject };

export interface StreamSink {
  shell(state: JsonObject): Promise<void>;
  boundary(name: string, mode?: 'final' | 'updatable', state?: JsonObject): Promise<void>;
  update(name: string, state: JsonObject): Promise<void>;
  finish(state?: JsonObject): Promise<void>;
}

export function acceptsWebUIStream(header: string | undefined): boolean {
  if (header === undefined) {
    return false;
  }
  for (const range of header.split(',')) {
    const [mediaType, ...parameters] = range.split(';');
    if (mediaType.trim().toLowerCase() !== WEBUI_STREAM_MEDIA_TYPE) {
      continue;
    }
    let quality = 1;
    for (const parameter of parameters) {
      const separator = parameter.indexOf('=');
      if (separator === -1 || parameter.slice(0, separator).trim().toLowerCase() !== 'q') {
        continue;
      }
      quality = Number(parameter.slice(separator + 1).trim());
      break;
    }
    if (Number.isFinite(quality) && quality > 0 && quality <= 1) {
      return true;
    }
  }
  return false;
}

export class WebUIStreamWriter implements StreamSink {
  readonly #response: ServerResponse;

  constructor(response: ServerResponse) {
    this.#response = response;
  }

  shell(state: JsonObject): Promise<void> {
    return this.#write({ type: 'shell', version: WEBUI_STREAM_VERSION, state });
  }

  boundary(
    name: string,
    mode: 'final' | 'updatable' = 'final',
    state?: JsonObject,
  ): Promise<void> {
    const record: StreamRecord = { type: 'boundary', name };
    if (mode === 'updatable') {
      record.mode = mode;
    }
    if (state !== undefined) {
      record.state = state;
    }
    return this.#write(record);
  }

  update(name: string, state: JsonObject): Promise<void> {
    return this.#write({ type: 'update', name, state });
  }

  async finish(state?: JsonObject): Promise<void> {
    const record: StreamRecord = { type: 'finish' };
    if (state !== undefined) {
      record.state = state;
    }
    await this.#write(record);
    this.#response.end();
  }

  async #write(record: StreamRecord): Promise<void> {
    const line = encodeRecord(record);
    if (this.#response.destroyed || this.#response.writableEnded) {
      throw new Error('WebUI API stream disconnected before the next record');
    }
    if (!this.#response.write(line, 'utf8')) {
      await waitForDrain(this.#response);
    }
  }
}

export function encodeRecord(record: StreamRecord): string {
  const json = JSON.stringify(record);
  if (json === undefined) {
    throw new Error('WebUI API stream record is not JSON serializable');
  }
  if (Buffer.byteLength(json) > MAX_RECORD_BYTES) {
    throw new Error(`WebUI API stream record exceeds ${MAX_RECORD_BYTES} bytes`);
  }
  return `${json}\n`;
}

function waitForDrain(response: ServerResponse): Promise<void> {
  return new Promise((resolve, reject) => {
    const cleanup = (): void => {
      response.off('drain', onDrain);
      response.off('close', onClose);
      response.off('error', onError);
    };
    const onDrain = (): void => {
      cleanup();
      resolve();
    };
    const onClose = (): void => {
      cleanup();
      reject(new Error('WebUI API stream disconnected while waiting for backpressure'));
    };
    const onError = (error: Error): void => {
      cleanup();
      reject(error);
    };
    response.once('drain', onDrain);
    response.once('close', onClose);
    response.once('error', onError);
  });
}
