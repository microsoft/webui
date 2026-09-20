// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { randomInt } from 'node:crypto';

import type { JsonObject } from './stream-protocol.js';

export interface FeedPost extends JsonObject {
  postId: string;
  author: string;
  text: string;
  likeCount: string;
}

export interface StreamingState extends JsonObject {
  feedBatch1: readonly FeedPost[];
  feedBatch2: readonly FeedPost[];
  feedBatch3: readonly FeedPost[];
}

export interface ForecastState extends JsonObject {
  location: string;
  temperature: string;
  condition: string;
  status: 'ready';
}

const FORECASTS: ReadonlyArray<readonly [temperature: string, condition: string]> = [
  ['68°F', 'Partly cloudy'],
  ['54°F', 'Light rain'],
  ['72°F', 'Clear'],
  ['61°F', 'Overcast'],
];

export const STREAMING_STATE: StreamingState = {
  feedBatch1: [
    {
      postId: '1',
      author: 'Ada',
      text: 'Streaming boundaries keep the composer interactive immediately.',
      likeCount: '4',
    },
    {
      postId: '2',
      author: 'Grace',
      text: 'This is feed batch one - it just committed.',
      likeCount: '1',
    },
  ],
  feedBatch2: [
    {
      postId: '3',
      author: 'Alan',
      text: 'Feed batch two arrived after its own checkpoint.',
      likeCount: '9',
    },
  ],
  feedBatch3: [
    {
      postId: '4',
      author: 'Barbara',
      text: 'Feed batch three is the lowest-priority chunk.',
      likeCount: '0',
    },
  ],
};

export function createForecast(): ForecastState {
  const [temperature, condition] = FORECASTS[randomInt(FORECASTS.length)];
  return {
    location: 'Redmond, WA',
    temperature,
    condition,
    status: 'ready',
  };
}
