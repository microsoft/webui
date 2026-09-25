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
    this.#evictOldest();
    const session = new TestSession();
    this.#sessions.set(id, session);
    return session;
  }

  existingSession(id: string): TestSession | undefined {
    return this.#sessions.get(id);
  }

  /**
   * Keep the session table bounded without ever refusing a new run.
   *
   * Playwright reuses an already-running API across suite runs, so this process
   * accumulates sessions for as long as a developer keeps it up. A hard cap
   * would start rejecting every new session after a few hundred tests, which
   * surfaces as unrelated tests failing on a 400 rather than as an obvious
   * capacity error. Map iteration is insertion-ordered, so the first key is the
   * oldest session.
   */
  #evictOldest(): void {
    while (this.#sessions.size >= MAX_SESSIONS) {
      const oldest = this.#sessions.keys().next();
      if (oldest.done === true) {
        return;
      }
      const evicted = this.#sessions.get(oldest.value);
      this.#sessions.delete(oldest.value);
      // An evicted session can still have a stream parked on one of its gates,
      // and nothing can reach it to release it once it leaves the table. Open
      // its gates on the way out so that stream finishes and gives back its
      // slot against --max-concurrent-streams.
      evicted?.releaseAll();
    }
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
