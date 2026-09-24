// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from "node:assert/strict";
import test from "node:test";
import { concernEvent, invalidateApproval } from "./webui-ai-review-invalidate.mjs";

const REPO = "microsoft/webui";
const HEAD = "a".repeat(40);
const APPROVAL = "AI - Approved";
const BEFORE = "2026-09-24T10:00:00Z";
const AFTER = "2026-09-24T11:00:00Z";

function event(name, overrides = {}) {
  const data = { repository: { full_name: REPO }, sender: { login: "reviewer" },
    action: "created", ...overrides };
  if (name === "pull_request_review") {
    return { ...data, action: "submitted", pull_request: { number: 42 },
      review: { id: 55, submitted_at: AFTER }, ...overrides };
  }
  if (name === "issue_comment") {
    return { ...data, issue: { number: 42, pull_request: {} },
      comment: { created_at: AFTER, updated_at: AFTER }, ...overrides };
  }
  return { ...data, pull_request: { number: 42 },
    comment: { created_at: AFTER, updated_at: AFTER }, ...overrides };
}

function client({ approvalHead = HEAD, approvalState = "APPROVED",
  approvedAt = BEFORE, label = true, prState = "open" } = {}) {
  const calls = [];
  return {
    calls,
    async pages(path) {
      calls.push(["GET", path]);
      if (path === "/issues/42/labels") return label ? [{ name: APPROVAL }] : [];
      if (path === "/pulls/42/reviews") {
        return [{ id: 17, state: approvalState, submitted_at: approvedAt,
          user: { login: "github-actions[bot]" },
          body: `Reviewed.\n\n<!-- webui-ai-review:${approvalHead}:approved -->` }];
      }
      throw new Error(`Unexpected page ${path}`);
    },
    async call(path, options = {}) {
      const method = options.method ?? "GET";
      calls.push([method, path]);
      if (path === "/pulls/42" && method === "GET") {
        return { number: 42, state: prState,
          base: { repo: { full_name: REPO } },
          head: { sha: HEAD, repo: { full_name: REPO } } };
      }
      if (path === `/issues/42/labels/${encodeURIComponent(APPROVAL)}` &&
          method === "DELETE") return null;
      throw new Error(`Unexpected call ${method} ${path}`);
    },
  };
}

test("new human comment on a PR invalidates a current green label", async () => {
  const api = client();
  assert.equal(await invalidateApproval("issue_comment", event("issue_comment"), api),
    true);
  assert.equal(api.calls.at(-1)[0], "DELETE");
});

test("staged invalidation previews a stale label without write permission", async () => {
  const api = client();
  assert.equal(await invalidateApproval("issue_comment", event("issue_comment"),
    api, true), true);
  assert.ok(api.calls.every(([method]) => method === "GET"));
});

test("new inline concern and submitted review invalidate approval", async () => {
  for (const name of ["pull_request_review_comment", "pull_request_review"]) {
    const api = client();
    assert.equal(await invalidateApproval(name, event(name), api), true);
    assert.equal(api.calls.at(-1)[0], "DELETE");
  }
});

test("a dismissed approval is invalidated even without a newer timestamp", async () => {
  const api = client({ approvalState: "DISMISSED" });
  const review = event("pull_request_review", { action: "dismissed",
    review: { id: 17, submitted_at: BEFORE }, pull_request: { number: 42 } });
  assert.equal(await invalidateApproval("pull_request_review", review, api), true);
  assert.equal(api.calls.at(-1)[0], "DELETE");
});

test("old comments, non-PR issues, own bot comments and absent labels do nothing", async () => {
  const old = event("issue_comment", { comment: {
    created_at: BEFORE, updated_at: BEFORE,
  } });
  const earlier = client();
  assert.equal(await invalidateApproval("issue_comment", old, earlier), false);
  assert.ok(earlier.calls.every(([method]) => method === "GET"));

  const issue = event("issue_comment", { issue: { number: 42 } });
  assert.equal(concernEvent("issue_comment", issue), null);
  const bot = event("issue_comment", { sender: { login: "github-actions[bot]" } });
  assert.equal(concernEvent("issue_comment", bot), null);
  const empty = client({ label: false });
  assert.equal(await invalidateApproval("issue_comment",
    event("issue_comment"), empty), false);
  assert.equal(empty.calls.length, 1);
});

test("head changes and dismissed approvals fail closed", async () => {
  for (const options of [{ approvalHead: "b".repeat(40) },
    { approvalState: "DISMISSED" }]) {
    const api = client(options);
    assert.equal(await invalidateApproval("issue_comment", event("issue_comment"),
      api), true);
    assert.equal(api.calls.at(-1)[0], "DELETE");
  }
});
