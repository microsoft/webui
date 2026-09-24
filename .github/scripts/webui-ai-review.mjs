// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import { ensureLabel, githubClient, LABELS, reconcileLabels, REPO } from "./webui-ai-review-api.mjs";

const REVIEWER = "mddinbox";
const APPROVABLE_AUTHOR = "mohamedmansour";
const BOT = "github-actions[bot]";
const SHA = /^[0-9a-f]{40}$/i;

function loginEqual(left, right) {
  return left?.toLowerCase() === right?.toLowerCase();
}

function marker(head, kind) {
  return `<!-- webui-ai-review:${head}:${kind} -->`;
}

function requiredCheck(checks) {
  const matches = checks.filter((check) => check.name === "PR Checks");
  const pending = matches.find((check) => check.status !== "completed");
  if (pending) return pending;
  if (matches.length > 1 && matches.some((check) =>
    !Number.isFinite(Date.parse(check.created_at ?? check.started_at)))) {
    return null;
  }
  return matches.sort((left, right) =>
    Date.parse(right.created_at ?? right.started_at) -
    Date.parse(left.created_at ?? left.started_at))[0];
}

function sameDiscussion(first, second) {
  return ["reviews", "threads", "issueComments"].every((key) =>
    JSON.stringify(first[key]) === JSON.stringify(second[key]));
}

function validNumber(value) {
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < 1) {
    throw new Error("Expected a positive pull request number.");
  }
  return number;
}

export function eventTarget(name, event) {
  if (event.repository?.full_name !== REPO) {
    throw new Error("This reviewer is restricted to microsoft/webui.");
  }
  if (name === "workflow_dispatch") {
    return { number: validNumber(event.inputs?.pull_request_number) };
  }
  if (name !== "pull_request_target") {
    throw new Error(`Unexpected trigger: ${name}`);
  }
  const pr = event.pull_request;
  if (!pr || pr.head?.repo?.full_name !== REPO ||
      pr.base?.repo?.full_name !== REPO) {
    return null;
  }
  if (!SHA.test(pr.head.sha) || !SHA.test(pr.base.sha)) {
    throw new Error("PR event lacks a pinned base/head revision.");
  }
  return { number: validNumber(pr.number), head: pr.head.sha, base: pr.base.sha };
}

export function readDecision(output, target) {
  if (!output || !Array.isArray(output.items) ||
      !Array.isArray(output.errors) || output.errors.length) {
    throw new Error("Agent output is incomplete or contains errors.");
  }
  const decisions = output.items.filter((item) => item?.type === "publish_webui_review");
  if (decisions.length !== 1) {
    throw new Error("Expected exactly one WebUI review decision.");
  }
  const item = decisions[0];
  const number = validNumber(item.pull_request_number);
  if (number !== target.number || !SHA.test(item.base_sha) ||
      !SHA.test(item.head_sha) ||
      (target.head && item.head_sha !== target.head) ||
      (target.base && item.base_sha !== target.base)) {
    throw new Error("Review decision does not match the triggering PR and revision.");
  }
  if (!["clean", "findings", "inconclusive", "unchanged"].includes(item.outcome) ||
      !["none", "confirmed", "rechecked"].includes(item.challenge) ||
      ![true, false, "true", "false"].includes(item.coverage_complete)) {
    throw new Error("Review outcome, challenge or coverage is invalid.");
  }
  const body = item.body ?? "";
  if (typeof body !== "string" || body.length > 30_000) {
    throw new Error("Review body is invalid or too long.");
  }
  let comments;
  try {
    comments = JSON.parse(item.comments_json || "[]");
  } catch {
    throw new Error("Review comments are not valid JSON.");
  }
  if (!Array.isArray(comments) || comments.length > 30 || comments.some((comment) =>
    !comment || typeof comment.path !== "string" ||
    !comment.path || comment.path.includes("..") ||
    !Number.isSafeInteger(comment.line) || comment.line < 1 ||
    typeof comment.body !== "string" || !comment.body.trim() ||
    comment.body.length > 10_000)) {
    throw new Error("Review comments must point to changed lines with actionable text.");
  }
  if (item.outcome !== "findings" &&
      (body || comments.length || item.challenge !== "none")) {
    throw new Error("A non-finding decision cannot include findings or a challenge.");
  }
  if (item.outcome === "findings" &&
      (!["confirmed", "rechecked"].includes(item.challenge) ||
        (!body.trim() && !comments.length))) {
    throw new Error("Findings require independent verification and actionable evidence.");
  }
  return { number, base: item.base_sha, head: item.head_sha,
    outcome: item.outcome,
    coverage: item.coverage_complete === true || item.coverage_complete === "true",
    body: body.trim(), comments };
}

export function chooseReview(decision, snapshot) {
  const { pr, reviews, threads, checks } = snapshot;
  if (pr.number !== decision.number || pr.head.sha !== decision.head ||
      pr.base.sha !== decision.base || pr.head.repo?.full_name !== REPO ||
      pr.base.repo?.full_name !== REPO) {
    throw new Error("PR scope or revision changed during review.");
  }
  if (pr.state !== "open" || pr.draft || loginEqual(pr.user?.login, BOT)) {
    return { event: null, label: null, reason: "closed, draft or bot-authored" };
  }
  if (!snapshot.discussionComplete) {
    return { event: null, label: null, reason: "discussion incomplete" };
  }
  const onHead = reviews.filter((review) => review.commit_id === decision.head);
  const botReview = onHead.find((review) =>
    loginEqual(review.user?.login, BOT) &&
    review.body?.includes(`<!-- webui-ai-review:${decision.head}:`));
  const botFindings = onHead.some((review) =>
    loginEqual(review.user?.login, BOT) &&
    review.body?.includes(marker(decision.head, "findings")));
  if (botFindings) {
    return { event: null, label: LABELS.findings.name,
      reason: "existing findings require a new head or human resolution" };
  }
  if (decision.outcome === "unchanged") {
    const check = requiredCheck(checks);
    const approvedAt = Date.parse(botReview?.submitted_at);
    const approved = botReview?.state === "APPROVED" &&
      botReview.body.includes(marker(decision.head, "approved")) &&
      Number.isFinite(approvedAt) &&
      !onHead.some((review) => review.state === "CHANGES_REQUESTED") &&
      check?.status === "completed" && check.conclusion === "success" &&
      !threads.some((thread) => !thread.isResolved &&
        thread.comments.some((comment) =>
          loginEqual(comment.author, REVIEWER) ||
          loginEqual(comment.author, BOT))) &&
      threads.every((thread) => thread.comments.every((comment) =>
        Number.isFinite(Date.parse(comment.updatedAt)) &&
        Date.parse(comment.updatedAt) <= approvedAt)) &&
      snapshot.issueComments.every((comment) =>
        Number.isFinite(Date.parse(comment.created_at)) &&
        Date.parse(comment.created_at) <= approvedAt);
    return { event: null,
      label: approved ? LABELS.approved.name : null,
      reason: "existing review on this head" };
  }
  if (decision.outcome === "inconclusive") {
    return { event: null, label: null, reason: "inconclusive" };
  }
  if (decision.outcome === "findings") {
    const previousApproval = botReview?.state === "APPROVED" ?
      "\n\nAn earlier automated approval on this head needs human review " +
      "and dismissal before merge." : "";
    return { event: "COMMENT", label: LABELS.findings.name,
      body: `${decision.body}${previousApproval}\n\n${marker(decision.head, "findings")}`,
      comments: decision.comments };
  }
  if (loginEqual(pr.user?.login, REVIEWER)) {
    return { event: null, label: null, reason: "human-authored PR" };
  }
  if (!decision.coverage || threads.some((thread) =>
    !thread.isResolved && thread.comments.some((comment) =>
      loginEqual(comment.author, REVIEWER) || loginEqual(comment.author, BOT)))) {
    return { event: null, label: null, reason: "coverage or threads incomplete" };
  }
  const required = requiredCheck(checks);
  if (required?.status !== "completed" || required.conclusion !== "success") {
    return { event: null, label: null, reason: "PR Checks not successful" };
  }
  if (onHead.some((review) => review.state === "CHANGES_REQUESTED")) {
    return { event: null, label: null, reason: "existing concerns on this head" };
  }
  if (botReview) {
    return { event: null,
      label: botReview.state === "APPROVED" &&
        botReview.body.includes(marker(decision.head, "approved")) ?
        LABELS.approved.name : null, reason: "already reviewed" };
  }
  if (onHead.some((review) =>
    loginEqual(review.user?.login, REVIEWER) && review.state === "APPROVED")) {
    return { event: null, label: null, reason: "human already reviewed this head" };
  }
  if (loginEqual(pr.user.login, APPROVABLE_AUTHOR)) {
    return { event: "APPROVE", label: LABELS.approved.name,
      body: `Reviewed ${decision.head.slice(0, 7)}; PR Checks succeeded.\n\n` +
        marker(decision.head, "approved"), comments: [] };
  }
  return { event: "COMMENT", label: null,
    body: `LGTM — no additional concerns on ${decision.head.slice(0, 7)}.\n\n` +
      marker(decision.head, "lgtm"), comments: [] };
}

function addedLines(patch) {
  const lines = new Set();
  let newLine = 0;
  for (const row of (patch ?? "").split("\n")) {
    if (row.startsWith("@@")) {
      const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(row);
      newLine = hunk ? Number(hunk[1]) : 0;
    } else if (newLine && row.startsWith("+") && !row.startsWith("+++")) {
      lines.add(newLine++);
    } else if (newLine && row.startsWith(" ")) {
      newLine++;
    }
  }
  return lines;
}

export function anchorFindings(action, files, head) {
  if (!action.comments?.length) return action;
  const changed = new Map(files.map((file) =>
    [file.filename, addedLines(file.patch)]));
  const inline = [];
  const bodyOnly = [];
  for (const comment of action.comments) {
    if (changed.get(comment.path)?.has(comment.line)) {
      inline.push(comment);
    } else {
      bodyOnly.push(`- ${comment.path}:${comment.line}: ${comment.body}`);
    }
  }
  if (!bodyOnly.length) return action;
  const tag = marker(head, "findings");
  return { ...action, comments: inline,
    body: action.body.replace(tag,
      `Inline unavailable (no changed-line anchor in GitHub's diff):\n` +
      `${bodyOnly.join("\n")}\n\n${tag}`) };
}

export async function phaseReview(eventName, event, client, staged, phase, result) {
  const target = eventTarget(eventName, event);
  if (!target) return false;
  const current = await client.call(`/pulls/${target.number}`);
  if (current.state !== "open" || current.draft ||
      current.head.sha !== (target.head ?? current.head.sha) ||
      current.base.sha !== (target.base ?? current.base.sha) ||
      current.head.repo?.full_name !== REPO ||
      current.base.repo?.full_name !== REPO) {
    return false;
  }
  if (phase !== "start" && phase !== "finish") {
    throw new Error("Unknown review phase.");
  }
  if (staged) {
    console.log(`STAGED #${target.number}: ${phase} — labels unchanged`);
  } else if (phase === "start" || result !== "success") {
    await reconcileLabels(client, target.number, null);
  }
  return true;
}

export async function waitForChecks(client, decision, pause = (ms) =>
  new Promise((resolve) => setTimeout(resolve, ms)), attempts = 90,
initial = null) {
  for (let attempt = 0; attempt < attempts; attempt++) {
    const snapshot = attempt === 0 && initial ?
      initial : await client.snapshot(decision.number, decision.head);
    if (snapshot.pr.base.sha !== decision.base) {
      throw new Error("PR base changed while waiting for checks.");
    }
    const check = requiredCheck(snapshot.checks);
    if (check?.status === "completed" || attempt === attempts - 1) {
      return snapshot;
    }
    await pause(30_000);
  }
  throw new Error("Required-check wait had no attempts.");
}

export async function processReview(eventName, event, output, client, staged) {
  const target = eventTarget(eventName, event);
  if (!target) return "Not a same-repository PR with a current review target.";
  const decision = readDecision(output, target);
  const before = await client.snapshot(target.number, decision.head);
  const initial = !staged && decision.outcome === "clean" ?
    await waitForChecks(client, decision, undefined, undefined, before) : before;
  if (!sameDiscussion(before, initial)) {
    throw new Error("Review discussion changed while waiting for CI.");
  }
  const action = chooseReview(decision, initial);
  if (staged) return `STAGED #${target.number}: ${action.event ?? "no review"}; ` +
    `label ${action.label ?? "none"}; ${action.reason ?? "validated"}`;
  if (action.event) {
    const prepared = action.event === "COMMENT" && action.comments?.length ?
      anchorFindings(action, await client.pages(`/pulls/${target.number}/files`),
        decision.head) : action;
    const fresh = await client.snapshot(target.number, decision.head);
    const next = chooseReview(decision, fresh);
    if (next.event !== action.event || next.body !== action.body ||
        next.label !== action.label || !sameDiscussion(initial, fresh)) {
      throw new Error("PR review decision changed before submission.");
    }
    const result = await client.call(`/pulls/${target.number}/reviews`, {
      method: "POST",
      body: JSON.stringify({
        event: prepared.event, body: prepared.body, commit_id: decision.head,
        comments: prepared.comments.map((comment) => ({
          ...comment, side: "RIGHT",
        })),
      }),
    });
    if (result.state !== (action.event === "APPROVE" ? "APPROVED" : "COMMENTED") ||
        result.commit_id !== decision.head || !loginEqual(result.user?.login, BOT) ||
        !Number.isSafeInteger(result.id) || result.id < 1 ||
        !Number.isFinite(Date.parse(result.submitted_at)) ||
        result.body !== prepared.body) {
      throw new Error("Published review receipt did not match the intended event/head.");
    }
  }
  if (action.label !== undefined) {
    const current = await client.call(`/pulls/${target.number}`);
    if (current.head.sha !== decision.head || current.base.sha !== decision.base ||
        current.state !== "open") {
      throw new Error("PR changed before label publication.");
    }
    if (action.label) await ensureLabel(client, action.label);
    await reconcileLabels(client, target.number, action.label);
  }
  return `#${target.number}: ${action.event ?? "no review"}; ${action.reason ?? "published"}`;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    if (process.env.GITHUB_REPOSITORY !== REPO) {
      throw new Error("Review publisher can only run in microsoft/webui.");
    }
    const phase = process.argv[2];
    if (!["start", "publish", "finish"].includes(phase)) {
      throw new Error("Expected a start, publish or finish phase.");
    }
    if (phase === "publish" && (process.env.WEBUI_DETECTION_SUCCESS !== "true" ||
        process.env.WEBUI_DETECTION_CONCLUSION !== "success")) {
      throw new Error("Threat detection did not complete successfully.");
    }
    const token = process.env.GH_TOKEN;
    const eventFile = process.env.GITHUB_EVENT_PATH;
    if (!token || !eventFile) {
      throw new Error("Missing GitHub event or API credential.");
    }
    const event = JSON.parse(await readFile(eventFile, "utf8"));
    const staged = process.env.WEBUI_AI_REVIEW_STAGED === "true" ||
      process.env.GH_AW_SAFE_OUTPUTS_STAGED === "true";
    const client = githubClient(token);
    if (phase === "publish") {
      if (!process.env.GH_AW_AGENT_OUTPUT) {
        throw new Error("Missing structured agent output.");
      }
      const output = JSON.parse(await readFile(process.env.GH_AW_AGENT_OUTPUT, "utf8"));
      console.log(await processReview(process.env.GITHUB_EVENT_NAME,
        event, output, client, staged));
    } else {
      const current = await phaseReview(process.env.GITHUB_EVENT_NAME, event,
        client, staged, phase, process.env.WEBUI_PUBLISH_RESULT);
      if (phase === "start" && process.env.GITHUB_OUTPUT) {
        const { appendFile } = await import("node:fs/promises");
        await appendFile(process.env.GITHUB_OUTPUT, `current=${current}\n`);
      }
    }
  } catch (error) {
    console.error(error);
    process.exitCode = 1;
  }
}
