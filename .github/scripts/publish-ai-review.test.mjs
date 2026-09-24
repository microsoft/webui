// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from "node:assert/strict";
import test from "node:test";
import { decideReview, parseReviewItem, publishFromOutput } from "./publish-ai-review.mjs";

const HEAD = "a".repeat(40);
const CHECKS = [{ name: "PR Checks", status: "completed", conclusion: "success" }];

function reviewItem(overrides = {}) {
  return parseReviewItem({
    pull_request_number: "42",
    reviewed_head: HEAD,
    outcome: "clean",
    coverage_complete: "true",
    challenge: "none",
    ...overrides,
  });
}

function candidate(overrides = {}) {
  return {
    item: reviewItem(),
    pr: {
      number: 42,
      state: "open",
      draft: false,
      user: { login: "mohamedmansour" },
      head: { sha: HEAD },
    },
    reviews: [],
    checks: CHECKS,
    threads: [],
    eligible: true,
    ...overrides,
  };
}

function mockClient({ author = "mohamedmansour", ci = CHECKS,
  priorReviews = [], existingLabels = [],
  changeHeadOnSecondRead = false } = {}) {
  const requests = [];
  let reads = 0;
  const pr = candidate().pr;
  pr.user.login = author;
  return {
    requests,
    async request(path, options = {}) {
      requests.push({ path, method: options.method ?? "GET", body: options.body });
      if (path === "/pulls/42" && !options.method) {
        reads++;
        return {
          ...pr,
          head: { sha: changeHeadOnSecondRead && reads > 1 ? "b".repeat(40) : HEAD },
        };
      }
      if (path.startsWith("/labels/")) {
        return { name: decodeURIComponent(path.slice("/labels/".length)) };
      }
      if (path === "/pulls/42/reviews" && options.method === "POST") {
        return {
          state: JSON.parse(options.body).event === "APPROVE" ? "APPROVED" : "COMMENTED",
        };
      }
      if (path === "/issues/42/labels" && options.method === "POST") {
        return [];
      }
      if (path.startsWith("/issues/42/labels/") && options.method === "DELETE") {
        return null;
      }
      throw new Error(`Unexpected request: ${options.method ?? "GET"} ${path}`);
    },
    async pages(path) {
      if (path === "/pulls/42/requested_reviewers") {
        return [{ login: "mddinbox" }];
      }
      if (path === "/pulls/42/reviews") return priorReviews;
      if (path === `/commits/${HEAD}/check-runs`) return ci;
      if (path === "/issues/42/labels") return existingLabels.map((name) => ({ name }));
      throw new Error(`Unexpected paginated request: ${path}`);
    },
    async reviewThreads() {
      return [];
    },
  };
}

test("only a clean Mohamed PR is approved", () => {
  const approval = decideReview(candidate());
  assert.equal(approval.event, "APPROVE");
  assert.equal(approval.label, "AI - Approved");
  assert.match(approval.body, /webui-ai-review:.*:approved/);
  const lgtm = decideReview(candidate({
    pr: { ...candidate().pr, user: { login: "another-author" } },
  }));
  assert.equal(lgtm.event, "COMMENT");
  assert.equal(lgtm.label, null);
  assert.match(lgtm.body, /^LGTM — no additional concerns on aaaaaaa\./);
  assert.match(lgtm.body, /webui-ai-review:.*:lgtm/);
});

test("self-authorship, draft, missing CI, stale head and unresolved threads fail closed", () => {
  assert.equal(decideReview(candidate({
    pr: { ...candidate().pr, user: { login: "mddinbox" } },
  })).event, null);
  assert.equal(decideReview(candidate({
    pr: { ...candidate().pr, draft: true },
  })).event, null);
  assert.throws(() => decideReview(candidate({ checks: [] })), /CI is not successful/);
  assert.throws(() => decideReview(candidate({
    pr: { ...candidate().pr, head: { sha: "b".repeat(40) } },
  })), /head changed/);
  assert.throws(() => decideReview(candidate({
    threads: [{ isResolved: false, comments: { nodes: [
      { author: { login: "mddinbox" } },
    ] } }],
  })), /unresolved threads/);
  assert.throws(() => decideReview(candidate({ eligible: false })), /not in the target/);
});

test("previous approval or change request on the head prevents another approval", () => {
  const byUser = { user: { login: "mddinbox" }, commit_id: HEAD };
  assert.equal(decideReview(candidate({
    reviews: [{ ...byUser, state: "APPROVED" }],
  })).event, null);
  assert.throws(() => decideReview(candidate({
    reviews: [{ ...byUser, state: "CHANGES_REQUESTED" }],
  })), /prior change request/);
  const bot = { user: { login: "github-actions[bot]" }, commit_id: HEAD,
    state: "APPROVED", body: `Reviewed.\n\n<!-- webui-ai-review:${HEAD}:approved -->` };
  assert.deepEqual(decideReview(candidate({ reviews: [bot] })), {
    event: null, label: "AI - Approved", reason: "already approved at head",
  });
  assert.equal(decideReview(candidate({
    reviews: [{ ...bot, body: "Another workflow approved this PR." }],
  })).label, undefined);
});

test("findings require independent challenge and valid inline data", () => {
  assert.throws(() => reviewItem({
    outcome: "findings", body: "Concrete issue", challenge: "none",
  }), /independent challenge/);
  assert.throws(() => reviewItem({
    outcome: "findings", challenge: "confirmed", inline_comments_json: "[null]",
  }), /invalid inline/);
  const item = reviewItem({
    outcome: "findings", challenge: "confirmed",
    body: "Trigger, mechanism, impact, and fix.",
  });
  assert.equal(decideReview(candidate({ item, checks: [] })).label,
    "AI - Changes Required");
  assert.throws(() => reviewItem({ body: "should be empty" }), /only findings/);
});

test("a rephrased finding on an unchanged head does not repost a review", () => {
  const old = { user: { login: "github-actions[bot]" }, commit_id: HEAD,
    state: "COMMENTED",
    body: `Original finding.\n\n<!-- webui-ai-review:${HEAD}:findings -->` };
  const item = reviewItem({
    outcome: "findings", challenge: "rechecked",
    body: "The original finding, rephrased.",
  });
  assert.deepEqual(decideReview(candidate({ item, reviews: [old] })), {
    event: null, label: "AI - Changes Required", reason: "duplicate findings",
  });
});

test("staged mode previews without a single GitHub write", async () => {
  const client = mockClient();
  await publishFromOutput({ items: [{ type: "publish_reviewed_pr",
    pull_request_number: "42", reviewed_head: HEAD, outcome: "clean",
    coverage_complete: true, challenge: "none" }] }, client, true);
  assert.equal(client.requests.filter((request) => request.method !== "GET").length, 0);
});

test("publisher submits an approval before applying its status label", async () => {
  const client = mockClient();
  await publishFromOutput({ items: [{ type: "publish_reviewed_pr",
    pull_request_number: "42", reviewed_head: HEAD, outcome: "clean",
    coverage_complete: true, challenge: "none" }] }, client, false);
  const writes = client.requests.filter((request) => request.method !== "GET");
  assert.equal(writes.length, 2);
  assert.equal(writes[0].path, "/pulls/42/reviews");
  assert.equal(JSON.parse(writes[0].body).event, "APPROVE");
  assert.equal(JSON.parse(writes[0].body).commit_id, HEAD);
  assert.equal(writes[1].path, "/issues/42/labels");
  assert.deepEqual(JSON.parse(writes[1].body).labels, ["AI - Approved"]);
});

test("publisher chooses LGTM for other authors and never assigns Approved label", async () => {
  const client = mockClient({ author: "another-author" });
  await publishFromOutput({ items: [{ type: "publish_reviewed_pr",
    pull_request_number: "42", reviewed_head: HEAD, outcome: "clean",
    coverage_complete: true, challenge: "none" }] }, client, false);
  const writes = client.requests.filter((request) => request.method !== "GET");
  assert.equal(writes.length, 1);
  assert.equal(JSON.parse(writes[0].body).event, "COMMENT");
  assert.match(JSON.parse(writes[0].body).body, /^LGTM/);
});

test("a finding sends COMMENT and replaces an obsolete Approved label", async () => {
  const client = mockClient({ ci: [], existingLabels: ["AI - Approved"] });
  await publishFromOutput({ items: [{ type: "publish_reviewed_pr",
    pull_request_number: "42", reviewed_head: HEAD, outcome: "findings",
    coverage_complete: false, challenge: "confirmed",
    body: "A changed line causes a reproducible defect; fix the root cause." }] },
  client, false);
  const writes = client.requests.filter((request) => request.method !== "GET");
  assert.equal(writes[0].path, "/pulls/42/reviews");
  assert.equal(JSON.parse(writes[0].body).event, "COMMENT");
  assert.equal(writes[1].method, "DELETE");
  assert.equal(decodeURIComponent(writes[1].path), "/issues/42/labels/AI - Approved");
  assert.deepEqual(JSON.parse(writes[2].body).labels, ["AI - Changes Required"]);
});

test("an inconclusive new head removes obsolete status labels without posting", async () => {
  const client = mockClient({ ci: [], existingLabels: ["AI - Approved"] });
  await publishFromOutput({ items: [{ type: "publish_reviewed_pr",
    pull_request_number: "42", reviewed_head: HEAD, outcome: "inconclusive",
    coverage_complete: false, challenge: "none" }] }, client, false);
  const writes = client.requests.filter((request) => request.method !== "GET");
  assert.deepEqual(writes.map((request) => request.method), ["DELETE"]);
});

test("inconclusive evidence revokes an approval label even on the reviewed head", () => {
  const previous = { user: { login: "mddinbox" }, commit_id: HEAD,
    state: "COMMENTED", body: "Prior review." };
  assert.deepEqual(decideReview(candidate({
    item: reviewItem({ outcome: "inconclusive", coverage_complete: false }),
    reviews: [previous],
  })), { event: null, label: null, reason: "inconclusive" });
});

test("failed CI cannot produce a clean review or a label", async () => {
  const client = mockClient({ ci: [{
    name: "PR Checks", status: "completed", conclusion: "failure",
  }] });
  await assert.rejects(publishFromOutput({ items: [{ type: "publish_reviewed_pr",
    pull_request_number: "42", reviewed_head: HEAD, outcome: "clean",
    coverage_complete: true, challenge: "none" }] }, client, false), /CI is not successful/);
  assert.equal(client.requests.filter((request) => request.method !== "GET").length, 0);
});

test("head changes before publication prevent approval and labeling", async () => {
  const client = mockClient({ changeHeadOnSecondRead: true });
  await assert.rejects(publishFromOutput({ items: [{ type: "publish_reviewed_pr",
    pull_request_number: "42", reviewed_head: HEAD, outcome: "clean",
    coverage_complete: true, challenge: "none" }] }, client, false), /target changed/);
  assert.equal(client.requests.filter((request) => request.method !== "GET").length, 0);
});

test("duplicate decisions for the same PR never publish twice", async () => {
  const client = mockClient();
  const item = { type: "publish_reviewed_pr", pull_request_number: "42",
    reviewed_head: HEAD, outcome: "clean", coverage_complete: true, challenge: "none" };
  await assert.rejects(publishFromOutput({ items: [item, item] }, client, false),
    /Duplicate decision/);
  assert.equal(client.requests.length, 0);
});
