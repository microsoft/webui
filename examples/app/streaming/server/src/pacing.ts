// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { randomInt } from 'node:crypto';

import { createForecast, STREAMING_STATE, type ForecastState } from './data.js';
import type { StreamSink } from './stream-protocol.js';
import type { TestSession } from './test-controls.js';

export const FEED_BATCH_COUNT = 3;
export const WEATHER_DELAY_MIN_MS = 700;

const WEATHER_DELAY_MAX_MS = 1_400;
const FEED_BOUNDARIES = ['feed-batch-1', 'feed-batch-2', 'feed-batch-3'] as const;

export interface PacingOptions {
  feedDelayMinMs: number;
  feedDelayMaxMs: number;
  testSession?: TestSession;
  signal?: AbortSignal;
}

type ReadyWork =
  | { type: 'weather'; forecast: ForecastState }
  | { type: 'feed' };

export async function streamPage(sink: StreamSink, options: PacingOptions): Promise<void> {
  await sink.shell(STREAMING_STATE);
  await sink.boundary('weather-shell', 'updatable');
  await sink.boundary('composer-ready');

  const weather = loadForecast(options.testSession, options.signal).then(
    (forecast): ReadyWork => ({ type: 'weather', forecast }),
  );
  let weatherPending = true;

  for (let batch = 0; batch < FEED_BATCH_COUNT; batch++) {
    const feed = waitForFeed(options, batch).then((): ReadyWork => ({ type: 'feed' }));
    if (weatherPending) {
      const ready = await Promise.race([weather, feed]);
      if (ready.type === 'weather') {
        await sink.update('weather-shell', ready.forecast);
        weatherPending = false;
        await feed;
      }
    } else {
      await feed;
    }
    await sink.boundary(FEED_BOUNDARIES[batch]);
  }

  if (weatherPending) {
    const ready = await weather;
    if (ready.type === 'weather') {
      await sink.update('weather-shell', ready.forecast);
    }
  }
  await sink.finish();
}

async function waitForFeed(options: PacingOptions, batch: number): Promise<void> {
  if (options.testSession) {
    await abortable(options.testSession.waitForFeedGap(batch), options.signal);
    return;
  }
  await sleep(randomDelay(options.feedDelayMinMs, options.feedDelayMaxMs), options.signal);
}

async function loadForecast(
  testSession?: TestSession,
  signal?: AbortSignal,
): Promise<ForecastState> {
  if (testSession) {
    await abortable(testSession.waitForWeather(), signal);
  } else {
    await sleep(randomDelay(WEATHER_DELAY_MIN_MS, WEATHER_DELAY_MAX_MS), signal);
  }
  return createForecast();
}

function randomDelay(minimum: number, maximum: number): number {
  if (maximum <= minimum) {
    return minimum;
  }
  return randomInt(minimum, maximum + 1);
}

function sleep(milliseconds: number, signal?: AbortSignal): Promise<void> {
  if (signal?.aborted) {
    return Promise.reject(abortError());
  }
  return new Promise((resolve, reject) => {
    const onAbort = (): void => {
      clearTimeout(timer);
      reject(abortError());
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort);
      resolve();
    }, milliseconds);
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}

function abortable<T>(operation: Promise<T>, signal?: AbortSignal): Promise<T> {
  if (!signal) {
    return operation;
  }
  if (signal.aborted) {
    return Promise.reject(abortError());
  }
  return new Promise((resolve, reject) => {
    const onAbort = (): void => {
      reject(abortError());
    };
    signal.addEventListener('abort', onAbort, { once: true });
    operation.then(
      (value) => {
        signal.removeEventListener('abort', onAbort);
        resolve(value);
      },
      (error: unknown) => {
        signal.removeEventListener('abort', onAbort);
        reject(error);
      },
    );
  });
}

function abortError(): Error {
  const error = new Error('streaming response disconnected');
  error.name = 'AbortError';
  return error;
}
