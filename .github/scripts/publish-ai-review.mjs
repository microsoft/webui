// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

const REPOSITORY = "microsoft/webui";
const TARGET_REVIEWER = "mddinbox";
const APPROVABLE_AUTHOR = "mohamedmansour";
const REVIEW_BOT = "github-actions[bot]";
const STATUS_LABELS = ["AI - Approved", "AI - Changes Required"];
const MAX_ITEMS = 100;
const MAX_PAGES = 20;

function sameLogin(first, second) {
  return first?.toLowerCase() === second?.toLowerCase();
}

function reviewMarker(head, outcome) {
  return `<!-- webui-ai-review:${head}:${outcome} -->`;
}

function requireValue(value, description) {
  if (!value) {
    throw new Error(description);
  }
  return value;
}

export function parseReviewItem(item) {
  const number = Number(item.pull_request_number);
  if (!Number.isSafeInteger(number) || number < 1) {
    throw new Error("Review output needs a positive PR number.");
  }
  const head = item.reviewed_head;
  if (typeof head !== "string" || !/^[0-9a-f]{40}$/i.test(head)) {
    throw new Error(`PR #${number}: reviewed_head must be a full SHA.`);
  }
  if (!["clean", "findings", "inconclusive"].includes(item.outcome)) {
    throw new Error(`PR #${number}: invalid review outcome.`);
  }
  if (!["confirmed", "rechecked", "none"].includes(item.challenge)) {
    throw new Error(`PR #${number}: invalid challenge result.`);
  }
  if (item.coverage_complete !== true && item.coverage_complete !== "true" &&
      item.coverage_complete !== false && item.coverage_complete !== "false") {
    throw new Error(`PR #${number}: coverage_complete must be a boolean.`);
  }
  const coverageComplete = item.coverage_complete === true ||
    item.coverage_complete === "true";
  const body = item.body ?? "";
  if (typeof body !== "string" || body.length > 30_000) {
    throw new Error(`PR #${number}: invalid review body.`);
  }
  let comments;
  try {
    comments = JSON.parse(item.inline_comments_json ?? "[]");
  } catch {
    throw new Error(`PR #${number}: inline_comments_json is not valid JSON.`);
  }
  if (!Array.isArray(comments) || comments.length > 30 ||
      comments.some((comment) =>
        !comment || typeof comment.path !== "string" || !comment.path ||
        comment.path.includes("..") ||
        !Number.isSafeInteger(comment.line) || comment.line < 1 ||
        typeof comment.body !== "string" || !comment.body.trim() ||
        comment.body.length > 10_000)) {
    throw new Error(`PR #${number}: invalid inline review comments.`);
  }
  if (item.outcome !== "findings" &&
      (comments.length || body || item.challenge !== "none")) {
    throw new Error(`PR #${number}: only findings may carry evidence or a challenge.`);
  }
  if (item.outcome === "findings" &&
      (!["confirmed", "rechecked"].includes(item.challenge) ||
        (!body.trim() && !comments.length))) {
    throw new Error(`PR #${number}: findings need an independent challenge and evidence.`);
  }
  return { number, head, outcome: item.outcome, coverageComplete, body, comments };
}

export function decideReview({ item, pr, reviews, checks, threads, eligible }) {
  if (!eligible) {
    throw new Error(`PR #${item.number}: not in the target review queue.`);
  }
  if (pr.head.sha !== item.head) {
    throw new Error(`PR #${item.number}: head changed during review.`);
  }
  if (pr.state !== "open" || pr.draft) {
    return { event: null, label: null, reason: "closed or draft" };
  }
  const ownThreads = threads.filter((thread) =>
    thread.comments.nodes.some((comment) =>
      sameLogin(comment.author?.login, TARGET_REVIEWER) ||
      sameLogin(comment.author?.login, REVIEW_BOT)));
  const currentReviews = reviews.filter((review) =>
    review.commit_id === item.head &&
    (sameLogin(review.user?.login, TARGET_REVIEWER) ||
      sameLogin(review.user?.login, REVIEW_BOT)));
  if (item.outcome === "inconclusive") {
    return { event: null, label: null, reason: "inconclusive" };
  }
  if (item.outcome === "findings") {
    if (currentReviews.some((review) =>
      sameLogin(review.user?.login, REVIEW_BOT) &&
      review.state === "COMMENTED" &&
      review.body?.includes(reviewMarker(item.head, "findings")))) {
      return {
        event: null,
        label: "AI - Changes Required",
        reason: "duplicate findings",
      };
    }
    return {
      event: "COMMENT",
      body: `${item.body.trim()}\n\n${reviewMarker(item.head, "findings")}`,
      comments: item.comments,
      label: "AI - Changes Required",
    };
  }
  if (!item.coverageComplete || ownThreads.some((thread) => !thread.isResolved)) {
    throw new Error(`PR #${item.number}: clean review has incomplete coverage or unresolved threads.`);
  }
  if (!checks.some((check) =>
    check.name === "PR Checks" && check.status === "completed" &&
    check.conclusion === "success") ||
      checks.some((check) => check.status !== "completed" ||
        ["failure", "cancelled", "timed_out", "action_required"].includes(check.conclusion))) {
    throw new Error(`PR #${item.number}: required or available CI is not successful.`);
  }
  if (sameLogin(pr.user.login, TARGET_REVIEWER) ||
      sameLogin(pr.user.login, REVIEW_BOT)) {
    return { event: null, label: null, reason: "self-authored PR" };
  }
  const priorApproval = currentReviews.find((review) => review.state === "APPROVED");
  if (priorApproval) {
    return {
      event: null,
      label: sameLogin(priorApproval.user?.login, REVIEW_BOT) &&
        priorApproval.body?.includes(reviewMarker(item.head, "approved")) ?
        "AI - Approved" : undefined,
      reason: "already approved at head",
    };
  }
  if (currentReviews.some((review) => review.state === "CHANGES_REQUESTED")) {
    throw new Error(`PR #${item.number}: a prior change request remains on this head.`);
  }
  if (sameLogin(pr.user.login, APPROVABLE_AUTHOR)) {
    return {
      event: "APPROVE",
      body: `Reviewed ${item.head.slice(0, 7)}; PR Checks succeeded.\n\n` +
        reviewMarker(item.head, "approved"),
      comments: [],
      label: "AI - Approved",
    };
  }
  const visibleBody = `LGTM — no additional concerns on ${item.head.slice(0, 7)}.`;
  const body = `${visibleBody}\n\n${reviewMarker(item.head, "lgtm")}`;
  if (currentReviews.some((review) =>
    review.state === "COMMENTED" &&
    (review.body === body || review.body === visibleBody))) {
    return { event: null, label: undefined, reason: "already LGTM at head" };
  }
  return { event: "COMMENT", body, comments: [], label: null };
}

function apiClient(token) {
  async function request(path, options = {}) {
    const response = await fetch(`https://api.github.com/repos/${REPOSITORY}${path}`, {
      ...options,
      headers: {
        Accept: "application/vnd.github+json",
        Authorization: `Bearer ${token}`,
        "X-GitHub-Api-Version": "2022-11-28",
        ...options.headers,
      },
    });
    if (!response.ok) {
      throw new Error(`${options.method ?? "GET"} ${path}: HTTP ${response.status}`);
    }
    return response.status === 204 ? null : response.json();
  }

  async function pages(path, extract = (value) => value) {
    const entries = [];
    for (let page = 1; page <= MAX_PAGES; page++) {
      const separator = path.includes("?") ? "&" : "?";
      const value = await request(`${path}${separator}per_page=100&page=${page}`);
      const chunk = extract(value);
      if (!Array.isArray(chunk)) {
        throw new Error(`${path}: unexpected pagination response.`);
      }
      entries.push(...chunk);
      if (chunk.length < 100) {
        return entries;
      }
    }
    throw new Error(`${path}: pagination exceeded ${MAX_PAGES} pages.`);
  }

  async function reviewThreads(number) {
    const threads = [];
    let cursor = null;
    for (let page = 1; page <= MAX_PAGES; page++) {
      const response = await fetch("https://api.github.com/graphql", {
        method: "POST",
        headers: {
          Accept: "application/vnd.github+json",
          Authorization: `Bearer ${token}`,
        },
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
                      nodes { author { login } }
                    }
                  }
                }
              }
            }
          }`,
          variables: { number, cursor },
        }),
      });
      if (!response.ok) {
        throw new Error(`PR #${number}: review thread read failed (HTTP ${response.status}).`);
      }
      const result = await response.json();
      const connection = result.data?.repository?.pullRequest?.reviewThreads;
      if (result.errors?.length || !connection ||
          connection.nodes.some((thread) => thread.comments.pageInfo.hasNextPage)) {
        throw new Error(`PR #${number}: review thread state incomplete.`);
      }
      threads.push(...connection.nodes);
      if (!connection.pageInfo.hasNextPage) {
        return threads;
      }
      cursor = requireValue(connection.pageInfo.endCursor, "Missing review thread cursor.");
    }
    throw new Error(`PR #${number}: review thread pagination incomplete.`);
  }

  return { request, pages, reviewThreads };
}

async function isEligible(client, number, reviews) {
  const requested = await client.pages(`/pulls/${number}/requested_reviewers`,
    (value) => value.users);
  if (requested.some((user) => sameLogin(user.login, TARGET_REVIEWER))) {
    return true;
  }
  if (reviews.some((review) =>
    sameLogin(review.user?.login, TARGET_REVIEWER) ||
    (sameLogin(review.user?.login, REVIEW_BOT) &&
      review.body?.includes("<!-- webui-ai-review:")))) {
    return true;
  }
  const [comments, reviewComments] = await Promise.all([
    client.pages(`/issues/${number}/comments`),
    client.pages(`/pulls/${number}/comments`),
  ]);
  return [...comments, ...reviewComments].some((comment) =>
    sameLogin(comment.user?.login, TARGET_REVIEWER));
}

async function labelsFor(client, number) {
  return client.pages(`/issues/${number}/labels`).then((labels) =>
    new Set(labels.map((label) => label.name)));
}

async function reconcileLabels(client, number, desired) {
  const existing = await labelsFor(client, number);
  for (const label of STATUS_LABELS) {
    if (label !== desired && existing.has(label)) {
      await client.request(`/issues/${number}/labels/${encodeURIComponent(label)}`, {
        method: "DELETE",
      });
    }
  }
  if (desired && !existing.has(desired)) {
    await client.request(`/issues/${number}/labels`, {
      method: "POST",
      body: JSON.stringify({ labels: [desired] }),
    });
  }
}

async function verifyStatusLabels(client) {
  for (const label of STATUS_LABELS) {
    await client.request(`/labels/${encodeURIComponent(label)}`);
  }
}

async function publishItem(client, item, staged) {
  const pr = await client.request(`/pulls/${item.number}`);
  const [reviews, checks, threads] = await Promise.all([
    client.pages(`/pulls/${item.number}/reviews`),
    client.pages(`/commits/${item.head}/check-runs`, (value) => value.check_runs),
    client.reviewThreads(item.number),
  ]);
  const eligible = await isEligible(client, item.number, reviews);
  const decision = decideReview({ item, pr, reviews, checks, threads, eligible });
  if (staged) {
    console.log(`STAGED PR #${item.number}: ${decision.event ?? "no review"}; ` +
      `label ${decision.label ?? "none"}; ${decision.reason ?? "validated"}`);
    return;
  }
  if (decision.event) {
    if (decision.label) {
      await verifyStatusLabels(client);
    }
    const latest = await client.request(`/pulls/${item.number}`);
    if (latest.head.sha !== item.head || latest.state !== "open" || latest.draft ||
        latest.user.login !== pr.user.login) {
      throw new Error(`PR #${item.number}: target changed before publication.`);
    }
    const currentChecks = await client.pages(`/commits/${item.head}/check-runs`,
      (value) => value.check_runs);
    const refreshed = decideReview({ item, pr: latest,
      reviews: await client.pages(`/pulls/${item.number}/reviews`),
      checks: currentChecks, threads: await client.reviewThreads(item.number), eligible });
    if (refreshed.event !== decision.event || refreshed.body !== decision.body ||
        refreshed.label !== decision.label) {
      throw new Error(`PR #${item.number}: review decision changed before publication.`);
    }
    const result = await client.request(`/pulls/${item.number}/reviews`, {
      method: "POST",
      body: JSON.stringify({
        commit_id: item.head,
        event: decision.event,
        body: decision.body,
        comments: decision.comments,
      }),
    });
    if (result.state !== (decision.event === "APPROVE" ? "APPROVED" : "COMMENTED")) {
      throw new Error(`PR #${item.number}: review state did not match ${decision.event}.`);
    }
    console.log(`PR #${item.number}: ${result.state} at ${item.head}.`);
  }
  if (decision.label !== undefined) {
    await reconcileLabels(client, item.number, decision.label);
  }
}

export async function publishFromOutput(output, client, staged) {
  if (!Array.isArray(output.items)) {
    throw new Error("Safe-output artifact has no items array.");
  }
  const items = output.items.filter((item) => item.type === "publish_reviewed_pr");
  if (items.length > MAX_ITEMS) {
    throw new Error(`More than ${MAX_ITEMS} PR review decisions in one run.`);
  }
  const seen = new Set();
  const parsed = items.map(parseReviewItem);
  for (const item of parsed) {
    if (seen.has(item.number)) {
      throw new Error(`Duplicate decision for PR #${item.number}.`);
    }
    seen.add(item.number);
  }
  for (const item of parsed) {
    await publishItem(client, item, staged);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    if (process.env.GITHUB_REPOSITORY !== REPOSITORY) {
      throw new Error(`Review publisher is restricted to ${REPOSITORY}.`);
    }
    if (!["schedule", "workflow_dispatch"].includes(process.env.GITHUB_EVENT_NAME)) {
      throw new Error("Review publisher requires a scheduled or manual run.");
    }
    if (process.env.WEBUI_DETECTION_SUCCESS !== "true" ||
        process.env.WEBUI_DETECTION_CONCLUSION !== "success") {
      throw new Error("Review publisher requires a clean threat-detection result.");
    }
    const token = requireValue(process.env.GH_TOKEN, "GH_TOKEN is required.");
    const outputPath = requireValue(process.env.GH_AW_AGENT_OUTPUT,
      "GH_AW_AGENT_OUTPUT is required.");
    const output = JSON.parse(await readFile(outputPath, "utf8"));
    const staged = process.env.GH_AW_SAFE_OUTPUTS_STAGED === "true" ||
      process.env.WEBUI_AI_REVIEW_STAGED === "true";
    await publishFromOutput(output, apiClient(token), staged);
  } catch (error) {
    console.error(error);
    process.exitCode = 1;
  }
}
