// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

const MAX_SESSION_ID_BYTES = 64;
const MAX_SESSIONS = 128;

interface FeedWaiter {
  required: number;
  resolve: () => void;
}

export class TestSession {
  #releasedFeedGaps = 0;
  #weatherReleased = false;
  readonly #feedWaiters: FeedWaiter[] = [];
  readonly #weatherWaiters: Array<() => void> = [];

  waitForFeedGap(batch: number): Promise<void> {
    const required = batch + 1;
    if (this.#releasedFeedGaps >= required) {
      return Promise.resolve();
    }
    return new Promise((resolve) => {
      this.#feedWaiters.push({ required, resolve });
    });
  }

  waitForWeather(): Promise<void> {
    if (this.#weatherReleased) {
      return Promise.resolve();
    }
    return new Promise((resolve) => {
      this.#weatherWaiters.push(resolve);
    });
  }

  releaseNextFeedGap(): void {
    this.#releasedFeedGaps++;
    this.#releaseFeedWaiters();
  }

  releaseWeather(): void {
    this.#weatherReleased = true;
    while (this.#weatherWaiters.length > 0) {
      this.#weatherWaiters.pop()?.();
    }
  }

  releaseAll(): void {
    this.#releasedFeedGaps = Number.MAX_SAFE_INTEGER;
    this.#releaseFeedWaiters();
    this.releaseWeather();
  }

  #releaseFeedWaiters(): void {
    let index = 0;
    while (index < this.#feedWaiters.length) {
      const waiter = this.#feedWaiters[index];
      if (waiter.required > this.#releasedFeedGaps) {
        index++;
        continue;
      }
      this.#feedWaiters.splice(index, 1);
      waiter.resolve();
    }
  }
}

export class TestControls {
  readonly #sessions = new Map<string, TestSession>();

  session(id: string): TestSession | undefined {
    if (!isValidSessionId(id)) {
      return undefined;
    }
    const existing = this.#sessions.get(id);
    if (existing) {
      return existing;
    }
    if (this.#sessions.size >= MAX_SESSIONS) {
      return undefined;
    }
    const session = new TestSession();
    this.#sessions.set(id, session);
    return session;
  }

  existingSession(id: string): TestSession | undefined {
    return this.#sessions.get(id);
  }
}

function isValidSessionId(id: string): boolean {
  if (id.length === 0 || Buffer.byteLength(id) > MAX_SESSION_ID_BYTES) {
    return false;
  }
  for (let index = 0; index < id.length; index++) {
    const code = id.charCodeAt(index);
    const alphaNumeric =
      (code >= 48 && code <= 57) ||
      (code >= 65 && code <= 90) ||
      (code >= 97 && code <= 122);
    if (!alphaNumeric && code !== 45 && code !== 95) {
      return false;
    }
  }
  return true;
}
