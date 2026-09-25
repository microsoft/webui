// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Map values with a fixed worker count while preserving input order.
 *
 * Workers stop taking new work after the first failure, then allow already
 * active operations to settle before the error is propagated.
 */
export async function mapConcurrent<T, U>(
  values: ReadonlyArray<T>,
  maxConcurrency: number,
  operation: (value: T, index: number, workerIndex: number) => Promise<U>
): Promise<U[]> {
  if (!Number.isSafeInteger(maxConcurrency) || maxConcurrency < 1) {
    throw new RangeError("maxConcurrency must be a positive safe integer");
  }

  const results = new Array<U>(values.length);
  const workerCount = Math.min(values.length, maxConcurrency);
  const workers = new Array<Promise<void>>(workerCount);
  let nextIndex = 0;
  let didFail = false;
  let failure: unknown;

  for (let workerIndex = 0; workerIndex < workerCount; workerIndex++) {
    workers[workerIndex] = (async () => {
      while (!didFail) {
        const index = nextIndex;
        if (index >= values.length) return;
        nextIndex++;
        try {
          results[index] = await operation(
            values[index]!,
            index,
            workerIndex
          );
        } catch (error: unknown) {
          if (!didFail) {
            didFail = true;
            failure = error;
          }
          return;
        }
      }
    })();
  }

  await Promise.all(workers);
  if (didFail) throw failure;
  return results;
}
