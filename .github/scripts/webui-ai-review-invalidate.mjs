// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import { githubClient, LABELS, REPO } from "./webui-ai-review-api.mjs";

const BOT = "github-actions[bot]";
const APPROVAL = LABELS.approved.name;

function timestamp(value) {
  const time = Date.parse(value);
  return Number.isFinite(time) ? time : null;
}

export function concernEvent(eventName, event) {
  if (event.repository?.full_name !== REPO ||
      !["created", "edited", "submitted", "dismissed"].includes(event.action)) {
    return null;
  }
  if (eventName === "pull_request_review") {
    if (!["submitted", "edited", "dismissed"].includes(event.action)) return null;
    if (event.sender?.login === BOT && event.action !== "dismissed") return null;
    const number = event.pull_request?.number;
    const when = timestamp(event.review?.updated_at ?? event.review?.submitted_at);
    if (!Number.isSafeInteger(number) || number < 1) return null;
    return { number, when, dismissedReviewId: event.action === "dismissed" ?
      event.review?.id : null };
  }
  if (eventName === "issue_comment" || eventName === "pull_request_review_comment") {
    if (!["created", "edited"].includes(event.action) ||
        event.sender?.login === BOT) return null;
    if (eventName === "issue_comment" && !event.issue?.pull_request) return null;
    const number = eventName === "issue_comment" ?
      event.issue?.number : event.pull_request?.number;
    if (!Number.isSafeInteger(number) || number < 1) return null;
    return { number, when: timestamp(event.comment?.updated_at ??
      event.comment?.created_at), dismissedReviewId: null };
  }
  return null;
}

export async function invalidateApproval(eventName, event, client, staged = false) {
  const concern = concernEvent(eventName, event);
  if (!concern) return false;
  const labels = await client.pages(`/issues/${concern.number}/labels`);
  if (!labels.some((label) => label.name === APPROVAL)) return false;
  const pr = await client.call(`/pulls/${concern.number}`);
  if (pr.state !== "open" || pr.head.repo?.full_name !== REPO ||
      pr.base.repo?.full_name !== REPO) return false;
  const reviews = await client.pages(`/pulls/${concern.number}/reviews`);
  const approval = reviews.filter((review) =>
    review.user?.login === BOT &&
    review.body?.includes(`<!-- webui-ai-review:${pr.head.sha}:approved -->`))
    .sort((left, right) =>
      Date.parse(right.submitted_at) - Date.parse(left.submitted_at))[0];
  const approvedAt = timestamp(approval?.submitted_at);
  if (approval?.state === "APPROVED" && approvedAt !== null &&
      concern.dismissedReviewId !== approval.id &&
      concern.when !== null && concern.when <= approvedAt) {
    return false;
  }
  if (staged) return true;
  // A 404 means a different run already removed the label.
  try {
    await client.call(`/issues/${concern.number}/labels/${encodeURIComponent(APPROVAL)}`,
      { method: "DELETE" });
  } catch (error) {
    if (!String(error.message).includes("HTTP 404")) throw error;
  }
  return true;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    if (process.env.GITHUB_REPOSITORY !== REPO ||
        !process.env.GH_TOKEN || !process.env.GITHUB_EVENT_PATH) {
      throw new Error("WebUI event, repository and read/write token are required.");
    }
    const event = JSON.parse(await readFile(process.env.GITHUB_EVENT_PATH, "utf8"));
    const changed = await invalidateApproval(process.env.GITHUB_EVENT_NAME, event,
      githubClient(process.env.GH_TOKEN),
      process.env.WEBUI_AI_REVIEW_STAGED === "true");
    console.log(changed ? process.env.WEBUI_AI_REVIEW_STAGED === "true" ?
      "STAGED: stale AI approval label would be removed." :
      "Stale AI approval label removed." :
      "No AI approval label invalidation needed.");
  } catch (error) {
    console.error(error);
    process.exitCode = 1;
  }
}
