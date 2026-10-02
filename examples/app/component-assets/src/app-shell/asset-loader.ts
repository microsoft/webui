// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export function createRetryableLoader<T>(load: () => Promise<T>): () => Promise<T> {
  let cached: Promise<T> | undefined;

  return () => {
    if (cached) return cached;

    const pending = load().catch(error => {
      if (cached === pending) cached = undefined;
      throw error;
    });
    cached = pending;
    return pending;
  };
}
