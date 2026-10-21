// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Bounds one UTF-8 record, excluding its LF terminator. */
export interface NdjsonReaderOptions {
  /** Defaults to 256 KiB. A coalesced read may contain several bounded records. */
  maxRecordBytes?: number;
  /** Aborting cancels the body and releases its reader immediately. */
  signal?: AbortSignal;
}

const DEFAULT_RECORD_BYTES = 256 * 1024;

/**
 * A bounded NDJSON reader independent of routing, resources, and DOM rendering.
 *
 * Retains at most one transport chunk and a reusable bounded record buffer.
 * Records are decoded once, including UTF-8 sequences split across reads.
 * Consumers own record validation and must cancel an abandoned response.
 */
export class NdjsonReader {
  private reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
  private chunk: Uint8Array | undefined;
  private offset = 0;
  private scratch: Uint8Array | undefined;
  private length = 0;
  private ended = false;
  private reading = false;
  private readonly limit: number;
  private readonly signal: AbortSignal | undefined;
  private readonly decoder = new TextDecoder('utf-8', { fatal: true });

  constructor(body: ReadableStream<Uint8Array>, options: NdjsonReaderOptions = {}) {
    this.limit = options.maxRecordBytes ?? DEFAULT_RECORD_BYTES;
    if (!Number.isSafeInteger(this.limit) || this.limit <= 0) {
      throw new RangeError('[WebUI] NDJSON maxRecordBytes must be a positive safe integer.');
    }
    this.signal = options.signal;
    this.signal?.throwIfAborted();
    this.reader = body.getReader();
    this.signal?.addEventListener('abort', this.onAbort, { once: true });
  }

  /** Read the next nonblank record, or undefined after a clean end of body. */
  async read(): Promise<unknown | undefined> {
    if (this.reading) throw new Error('[WebUI] Await the current NDJSON read before starting another.');
    this.reading = true;
    try {
      while (true) {
        this.signal?.throwIfAborted();
        const record = this.takeBuffered();
        if (record !== undefined || this.ended) return record;
        const reader = this.reader;
        if (!reader) return undefined;
        const result = await reader.read();
        this.signal?.throwIfAborted();
        if (result.done) {
          this.ended = true;
          this.releaseReader();
        } else {
          this.chunk = result.value;
          this.offset = 0;
        }
      }
    } catch (error) {
      try {
        await this.cancel(error);
      } catch (cleanupError) {
        throw new AggregateError([error, cleanupError], '[WebUI] NDJSON reading and cancellation failed.');
      }
      throw error;
    } finally {
      this.reading = false;
    }
  }

  /**
   * Consume a complete record already received, without requesting more bytes.
   * Returns undefined when no complete record is buffered.
   */
  readBuffered(): unknown | undefined {
    if (this.reading) throw new Error('[WebUI] Await the current NDJSON read before consuming buffered records.');
    try {
      return this.takeBuffered();
    } catch (error) {
      void this.cancel(error).catch(cleanupError => {
        console.warn('[WebUI] Failed to cancel an invalid NDJSON body:', cleanupError);
      });
      throw error;
    }
  }

  private takeBuffered(): unknown | undefined {
    while (this.chunk) {
      const chunk = this.chunk;
      const newline = chunk.indexOf(10, this.offset);
      const end = newline === -1 ? chunk.length : newline;
      const part = chunk.subarray(this.offset, end);
      const size = this.length + part.length;
      if (size > this.limit) throw this.recordLimit();
      this.offset = end + 1;
      if (this.offset >= chunk.length) {
        this.chunk = undefined;
        this.offset = 0;
      }
      if (newline === -1) {
        this.append(part);
        break;
      }
      let record: unknown;
      if (this.length > 0) {
        this.append(part);
        record = this.decode(this.scratch!.subarray(0, this.length));
        this.length = 0;
      } else {
        record = this.decode(part);
      }
      if (record !== undefined) return record;
    }
    if (this.ended && this.length > 0) {
      const record = this.decode(this.scratch!.subarray(0, this.length));
      this.length = 0;
      this.scratch = undefined;
      return record;
    }
    if (this.ended) this.scratch = undefined;
    return undefined;
  }

  /** Release buffered bytes and cancel an unread body. Safe to call repeatedly. */
  async cancel(reason?: unknown): Promise<void> {
    const reader = this.reader;
    this.ended = true;
    this.chunk = undefined;
    this.scratch = undefined;
    this.length = 0;
    this.offset = 0;
    if (!reader) return;
    try {
      const cancellation = reader.cancel(reason);
      this.releaseReader();
      await cancellation;
    } finally {
      this.releaseReader();
    }
  }

  private readonly onAbort = (): void => {
    void this.cancel(this.signal?.reason).catch(error => {
      console.warn('[WebUI] Failed to cancel an aborted NDJSON body:', error);
    });
  };

  private releaseReader(): void {
    this.signal?.removeEventListener('abort', this.onAbort);
    this.reader?.releaseLock();
    this.reader = undefined;
  }

  private append(part: Uint8Array): void {
    if (part.length === 0) return;
    const size = this.length + part.length;
    if (size > this.limit) throw this.recordLimit();
    if (!this.scratch || this.scratch.length < size) {
      const capacity = Math.min(this.limit, Math.max(size, (this.scratch?.length ?? 1024) * 2));
      const next = new Uint8Array(capacity);
      if (this.scratch) next.set(this.scratch.subarray(0, this.length));
      this.scratch = next;
    }
    this.scratch.set(part, this.length);
    this.length = size;
  }

  private decode(bytes: Uint8Array): unknown {
    let text: string;
    try {
      text = this.decoder.decode(bytes);
    } catch (cause) {
      throw new Error('[WebUI] Invalid UTF-8 in NDJSON response. Send UTF-8 encoded JSON records.', { cause });
    }
    if (!text.trim()) return undefined;
    try {
      return JSON.parse(text);
    } catch (cause) {
      throw new Error('[WebUI] Invalid NDJSON record. Send one complete JSON value per line.', { cause });
    }
  }

  private recordLimit(): RangeError {
    return new RangeError(
      `[WebUI] NDJSON record exceeds ${this.limit} bytes. Reduce the response record or select an explicit larger budget.`,
    );
  }
}
