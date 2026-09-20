// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { TestControls } from './test-controls.js';

const MAX_SESSIONS = 128;

test('reuses the session registered under an id', () => {
  const controls = new TestControls();
  const first = controls.session('alpha');
  assert.ok(first);
  assert.equal(controls.session('alpha'), first);
  assert.equal(controls.existingSession('alpha'), first);
});

test('rejects ids that are empty or contain unsupported characters', () => {
  const controls = new TestControls();
  assert.equal(controls.session(''), undefined);
  assert.equal(controls.session('has space'), undefined);
  assert.equal(controls.session('has/slash'), undefined);
  assert.equal(controls.session('a'.repeat(65)), undefined);
  assert.ok(controls.session('ok-id_9'));
});

test('never refuses a new session once the table is full', () => {
  const controls = new TestControls();
  for (let index = 0; index < MAX_SESSIONS; index++) {
    assert.ok(controls.session(`fill-${index}`));
  }

  // Playwright reuses a running API across suite runs, so exhausting the table
  // must not start failing every later test with a 400.
  const overflow = controls.session('overflow');
  assert.ok(overflow, 'a new session is still handed out when the table is full');
  assert.equal(controls.existingSession('overflow'), overflow);
});

test('evicts the oldest session first and keeps the newest', () => {
  const controls = new TestControls();
  for (let index = 0; index < MAX_SESSIONS; index++) {
    controls.session(`fill-${index}`);
  }
  const newest = controls.existingSession(`fill-${MAX_SESSIONS - 1}`);

  controls.session('overflow');

  assert.equal(controls.existingSession('fill-0'), undefined, 'oldest session was evicted');
  assert.equal(
    controls.existingSession(`fill-${MAX_SESSIONS - 1}`),
    newest,
    'newest session survived',
  );
});

test('opens the gates of an evicted session so its stream can finish', async () => {
  const controls = new TestControls();
  const doomed = controls.session('doomed');
  assert.ok(doomed);

  // A stream parked on this gate becomes unreachable once the session leaves
  // the table, so eviction must release it or it holds its concurrency slot
  // for the life of the process. Race a timeout so a regression fails here
  // instead of hanging the run.
  const parked = Promise.all([doomed.waitForFeedGap(0), doomed.waitForWeather()]).then(
    () => 'released' as const,
  );
  const timeout = new Promise<'stuck'>((resolve) => {
    setTimeout(() => resolve('stuck'), 1_000).unref();
  });

  for (let index = 0; index < MAX_SESSIONS; index++) {
    controls.session(`fill-${index}`);
  }

  assert.equal(await Promise.race([parked, timeout]), 'released');
  assert.equal(controls.existingSession('doomed'), undefined);
});
