// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Fetch one overlay's fresh state from the sample API. */
export async function fetchState(
  region: 'activity' | 'settings',
  slow: boolean,
  signal: AbortSignal,
): Promise<Record<string, unknown>> {
  // Relative to the document base, so the app also works under a demo subdirectory.
  const url = new URL(`api/${region}`, document.baseURI);
  if (slow) url.searchParams.set('slow', '1');
  const response = await fetch(url, { signal });
  if (!response.ok) throw new Error(`State request failed: ${response.status}`);
  const body = await response.json() as { state?: unknown };
  if (!body.state || typeof body.state !== 'object') throw new Error('The sample API returned no state.');
  return body.state as Record<string, unknown>;
}
