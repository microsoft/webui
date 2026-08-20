// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { STREAMING_STATE } from './data.js';
import { streamPage } from './pacing.js';
import {
  acceptsWebUIStream,
  type BoundaryTarget,
  type StreamSink,
} from './stream-protocol.js';
import { TestControls } from './test-controls.js';

interface RecordedCommand {
  type: 'start' | 'resume' | 'update';
  boundary?: BoundaryTarget;
}

class RecordingSink implements StreamSink {
  readonly commands: RecordedCommand[] = [];
  #changed: (() => void) | undefined;

  start(): Promise<void> {
    return this.#record({ type: 'start' });
  }

  resume(boundary: BoundaryTarget): Promise<void> {
    return this.#record({ type: 'resume', boundary });
  }

  update(boundary: BoundaryTarget): Promise<void> {
    return this.#record({ type: 'update', boundary });
  }

  async waitFor(type: RecordedCommand['type'], name?: string): Promise<void> {
    while (
      !this.commands.some(
        (command) => command.type === type && command.boundary?.name === name,
      )
    ) {
      await new Promise<void>((resolve) => {
        this.#changed = resolve;
      });
    }
  }

  #record(command: RecordedCommand): Promise<void> {
    this.commands.push(command);
    this.#changed?.();
    this.#changed = undefined;
    return Promise.resolve();
  }
}

test('ready weather can arrive between feed boundaries', async () => {
  const controls = new TestControls();
  const session = controls.session('interleaved');
  assert.ok(session);
  const sink = new RecordingSink();
  const streaming = streamPage(sink, {
    feedDelayMinMs: 0,
    feedDelayMaxMs: 0,
    testSession: session,
  });

  await sink.waitFor('resume', 'composer-ready');
  session.releaseNextFeedGap();
  await sink.waitFor('resume', 'feed-batch-1');
  session.releaseWeather();
  await sink.waitFor('update', 'weather-shell');
  assert.equal(
    sink.commands.some((command) => command.boundary?.name === 'feed-batch-2'),
    false,
  );

  session.releaseAll();
  await streaming;
  assert.deepEqual(
    sink.commands
      .filter((command) => command.type === 'resume')
      .map((command) => command.boundary?.name),
    [
      'weather-shell',
      'composer-ready',
      'feed-batch-1',
      'feed-batch-2',
      'feed-batch-3',
    ],
  );
  assert.equal(sink.commands.at(-1)?.boundary?.name, 'feed-batch-3');
  assert.equal(
    sink.commands
      .filter((command) => command.boundary)
      .every((command) => command.boundary?.owner === 'streaming-page'),
    true,
  );
});

test('a late weather update is sent before the final resume completes the stream', async () => {
  const controls = new TestControls();
  const session = controls.session('late-weather');
  assert.ok(session);
  const sink = new RecordingSink();
  const streaming = streamPage(sink, {
    feedDelayMinMs: 0,
    feedDelayMaxMs: 0,
    testSession: session,
  });

  await sink.waitFor('resume', 'composer-ready');
  session.releaseNextFeedGap();
  await sink.waitFor('resume', 'feed-batch-1');
  session.releaseNextFeedGap();
  await sink.waitFor('resume', 'feed-batch-2');
  session.releaseNextFeedGap();
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(
    sink.commands.some((command) => command.boundary?.name === 'feed-batch-3'),
    false,
  );

  session.releaseWeather();
  await streaming;
  const update = sink.commands.findIndex((command) => command.type === 'update');
  const finalResume = sink.commands.findIndex(
    (command) => command.boundary?.name === 'feed-batch-3',
  );
  assert.ok(update >= 0);
  assert.ok(finalResume > update);
});

test('every feed post carries the fields bound by feed-item', () => {
  for (const batch of [
    STREAMING_STATE.feedBatch1,
    STREAMING_STATE.feedBatch2,
    STREAMING_STATE.feedBatch3,
  ]) {
    for (const post of batch) {
      for (const field of ['postId', 'author', 'text', 'likeCount'] as const) {
        assert.notEqual(post[field], '');
      }
    }
  }
});

test('stream negotiation rejects explicitly unacceptable media ranges', () => {
  assert.equal(
    acceptsWebUIStream('application/x-webui-stream;q=0, application/json'),
    false,
  );
  assert.equal(
    acceptsWebUIStream('application/json, APPLICATION/X-WEBUI-STREAM; q=0.5'),
    true,
  );
});

test('disconnect cancellation stops before the next paced boundary', async () => {
  const controls = new TestControls();
  const session = controls.session('cancelled');
  assert.ok(session);
  const sink = new RecordingSink();
  const abort = new AbortController();
  const streaming = streamPage(sink, {
    feedDelayMinMs: 0,
    feedDelayMaxMs: 0,
    testSession: session,
    signal: abort.signal,
  });

  await sink.waitFor('resume', 'composer-ready');
  abort.abort();
  await assert.rejects(streaming, { name: 'AbortError' });
  assert.equal(
    sink.commands.some((command) => command.boundary?.name === 'feed-batch-1'),
    false,
  );
  session.releaseAll();
});
