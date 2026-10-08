// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export const REPO = "microsoft/webui";
export const LABELS = {
  approved: { name: "AI - Approved", color: "0e8a16",
    description: "AI review approved the current WebUI PR head" },
  findings: { name: "AI - Changes Required", color: "d93f0b",
    description: "AI review found actionable concerns on the current PR head" },
};

export function githubClient(token) {
  async function call(path, options = {}) {
    const response = await fetch(`https://api.github.com/repos/${REPO}${path}`, {
      ...options,
      headers: { Accept: "application/vnd.github+json",
        Authorization: `Bearer ${token}`, "X-GitHub-Api-Version": "2022-11-28",
        ...options.headers },
      signal: AbortSignal.timeout(30_000),
    });
    if (!response.ok) {
      throw new Error(`${options.method ?? "GET"} ${path}: HTTP ${response.status}`);
    }
    return response.status === 204 ? null : response.json();
  }

  async function pages(path, select = (value) => value) {
    const all = [];
    for (let page = 1; page <= 10; page++) {
      const value = await call(`${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`);
      const rows = select(value);
      if (!Array.isArray(rows)) throw new Error(`Incomplete page: ${path}`);
      all.push(...rows);
      if (rows.length < 100) return all;
    }
    throw new Error(`Pagination limit exceeded: ${path}`);
  }

  async function discussion(number) {
    const threads = [];
    let cursor = null;
    for (let page = 0; page < 10; page++) {
      const response = await fetch("https://api.github.com/graphql", {
        method: "POST",
        headers: { Accept: "application/vnd.github+json",
          Authorization: `Bearer ${token}` },
        body: JSON.stringify({
          query: `query($number:Int!,$cursor:String) {
            repository(owner:"microsoft",name:"webui") {
              pullRequest(number:$number) {
                reviewThreads(first:100,after:$cursor) {
                  pageInfo { hasNextPage endCursor }
                  nodes {
                    isResolved
                    comments(first:100) {
                      pageInfo { hasNextPage }
                      nodes { author { login } updatedAt }
                    }
                  }
                }
              }
            }
          }`,
          variables: { number, cursor },
        }),
        signal: AbortSignal.timeout(30_000),
      });
      if (!response.ok) throw new Error(`Thread read: HTTP ${response.status}`);
      const data = await response.json();
      const connection = data.data?.repository?.pullRequest?.reviewThreads;
      if (data.errors?.length || !connection || !Array.isArray(connection.nodes) ||
          typeof connection.pageInfo?.hasNextPage !== "boolean" ||
          connection.nodes.some((thread) => typeof thread.isResolved !== "boolean" ||
            !Array.isArray(thread.comments?.nodes) ||
            thread.comments.pageInfo?.hasNextPage !== false)) {
        throw new Error("Review discussion cannot be read completely.");
      }
      for (const thread of connection.nodes) {
        threads.push({ isResolved: thread.isResolved,
          comments: thread.comments.nodes.map((comment) => ({
            author: comment.author?.login, updatedAt: comment.updatedAt,
          })) });
      }
      if (!connection.pageInfo.hasNextPage) return threads;
      cursor = connection.pageInfo.endCursor;
      if (!cursor) throw new Error("Review thread cursor missing.");
    }
    throw new Error("Review thread pagination limit exceeded.");
  }

  async function snapshot(number, head) {
    const pr = await call(`/pulls/${number}`);
    if (pr.head.sha !== head) throw new Error("PR head changed during verification.");
    const [reviews, threads, issueComments, checks] = await Promise.all([
      pages(`/pulls/${number}/reviews`),
      discussion(number),
      pages(`/issues/${number}/comments`),
      pages(`/commits/${head}/check-runs`, (value) => value.check_runs),
    ]);
    return { pr, reviews, threads, issueComments, checks, discussionComplete: true };
  }
  return { call, pages, snapshot };
}

export async function reconcileLabels(client, number, desired) {
  const existing = new Set((await client.pages(`/issues/${number}/labels`))
    .map((label) => label.name));
  for (const label of Object.values(LABELS)) {
    if (label.name !== desired && existing.has(label.name)) {
      try {
        await client.call(`/issues/${number}/labels/${encodeURIComponent(label.name)}`,
          { method: "DELETE" });
      } catch (error) {
        if (!String(error.message).includes("HTTP 404")) throw error;
      }
    }
  }
  if (desired && !existing.has(desired)) {
    await client.call(`/issues/${number}/labels`,
      { method: "POST", body: JSON.stringify({ labels: [desired] }) });
  }
}

export async function ensureLabel(client, name) {
  const label = Object.values(LABELS).find((candidate) => candidate.name === name);
  if (!label) throw new Error("Unknown WebUI AI review label.");
  try {
    await client.call(`/labels/${encodeURIComponent(name)}`);
  } catch (error) {
    if (!String(error.message).includes("HTTP 404")) throw error;
    try {
      await client.call("/labels", { method: "POST", body: JSON.stringify(label) });
    } catch (creationError) {
      if (!String(creationError.message).includes("HTTP 422")) throw creationError;
      await client.call(`/labels/${encodeURIComponent(name)}`);
    }
  }
}
