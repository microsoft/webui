// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from "node:assert/strict";
import test from "node:test";
import { githubClient } from "./webui-ai-review-api.mjs";
import { anchorFindings, chooseReview, eventTarget, phaseReview, processReview, readDecision,
  waitForChecks } from "./webui-ai-review.mjs";

const REPO = "microsoft/webui";
const HEAD = "a".repeat(40);
const BASE = "b".repeat(40);

function pr(author = "mohamedmansour") {
  return { number: 42, state: "open", draft: false, user: { login: author },
    base: { sha: BASE, repo: { full_name: REPO } },
    head: { sha: HEAD, repo: { full_name: REPO } } };
}

function event(name = "pull_request_target", author = "mohamedmansour") {
  const repository = { full_name: REPO };
  if (name === "workflow_dispatch") {
    return { repository, inputs: { pull_request_number: "42" } };
  }
  return { repository, pull_request: pr(author) };
}

function decision(overrides = {}) {
  return { type: "publish_webui_review", pull_request_number: 42,
    base_sha: BASE, head_sha: HEAD, outcome: "clean",
    coverage_complete: true, challenge: "none",
    challenge_report: undefined, ...overrides };
}

function confirmedFinding(overrides = {}) {
  return decision({ outcome: "findings", coverage_complete: false,
    challenge: "confirmed", body: "Concrete trigger and smallest fix.",
    challenge_report: JSON.stringify({ name: "webui-findings-challenger",
      base_sha: BASE, head_sha: HEAD,
      findings: [{ verdict: "confirm", location: "DESIGN.md:10",
        evidence: "DESIGN.md:42 and base:38" },
        { verdict: "confirm", location: "DESIGN.md:42",
          evidence: "DESIGN.md:42 and base:38" }] }),
    ...overrides });
}

function output(item = decision()) {
  return { items: [item], errors: [] };
}

function checks(conclusion = "success") {
  return [{ name: "PR Checks", status: "completed", conclusion,
    app: { slug: "github-actions" },
    started_at: "2026-09-24T10:00:00Z" }];
}

function snapshot(author = "mohamedmansour", overrides = {}) {
  return { pr: pr(author), reviews: [], threads: [], issueComments: [], checks: checks(),
    discussionComplete: true, ...overrides };
}

function mockClient({ author = "mohamedmansour", currentChecks = checks(),
  currentReviews = [], currentThreads = [], existingLabels = [],
  postedState = null, receiptId = 123, labelMissing = false,
  changedHeadOnSecondRead = false } = {}) {
  const requests = [];
  let reads = 0;
  const client = {
    requests,
    async call(path, options = {}) {
      const method = options.method ?? "GET";
      requests.push({ path, method, body: options.body });
      if (path === "/pulls/42" && method === "GET") {
        reads++;
        const value = pr(author);
        if (changedHeadOnSecondRead && reads > 1) value.head.sha = "c".repeat(40);
        return value;
      }
      if (path === "/pulls/42/reviews" && method === "POST") {
        const requested = JSON.parse(options.body);
        const receipt = { state: postedState ?? (requested.event === "APPROVE" ?
          "APPROVED" : "COMMENTED"), commit_id: requested.commit_id,
        id: receiptId, submitted_at: "2026-09-24T11:00:00Z",
        body: requested.body,
        user: { login: "github-actions[bot]" } };
        currentReviews.push(receipt);
        return receipt;
      }
      if (path.startsWith("/labels/") && method === "GET") {
        if (labelMissing) throw new Error(`${method} ${path}: HTTP 404`);
        return { name: path };
      }
      if (path === "/labels" && method === "POST") return { name: "created" };
      if (path.startsWith("/issues/42/labels/") && method === "DELETE") return null;
      if (path === "/issues/42/labels" && method === "POST") return [];
      throw new Error(`Unexpected call: ${method} ${path}`);
    },
    async pages(path) {
      if (path === "/issues/42/labels") {
        return existingLabels.map((name) => ({ name }));
      }
      if (path === "/pulls/42/files") {
        return [{ filename: "DESIGN.md",
          patch: "@@ -10,2 +10,2 @@\n-old\n+new" }];
      }
      throw new Error(`Unexpected pages: ${path}`);
    },
    async snapshot() {
      const value = await client.call("/pulls/42");
      return { pr: value, reviews: currentReviews, threads: currentThreads,
        issueComments: [],
        checks: currentChecks, discussionComplete: true };
    },
  };
  return client;
}

function writes(client) {
  return client.requests.filter((request) => request.method !== "GET");
}

test("PR events bind a single same-repository head; forks are skipped", () => {
  assert.deepEqual(eventTarget("pull_request_target", event()),
    { number: 42, base: BASE, head: HEAD });
  const fork = event();
  fork.pull_request.head.repo.full_name = "contributor/webui";
  assert.equal(eventTarget("pull_request_target", fork), null);
  assert.deepEqual(eventTarget("workflow_dispatch", event("workflow_dispatch")),
    { number: 42 });
  assert.throws(() => eventTarget("schedule", event()), /Unexpected trigger/);
});

test("structured decisions must target the event and have valid evidence", () => {
  const target = eventTarget("pull_request_target", event());
  assert.equal(readDecision(output(), target).head, HEAD);
  assert.throws(() => readDecision(output(decision({ head_sha: "c".repeat(40) })),
    target), /does not match/);
  assert.throws(() => readDecision({ items: [decision(), decision()], errors: [] },
    target), /exactly one/);
  assert.throws(() => readDecision({ items: [decision()], errors: ["bad"] },
    target), /incomplete/);
  assert.throws(() => readDecision(output(decision({
    outcome: "findings", body: "Known defect", challenge: "none",
  })), target), /independent verification/);
  assert.throws(() => readDecision(output(confirmedFinding({
    comments_json: "[null]",
  })), target), /changed lines/);
  assert.throws(() => readDecision(output(confirmedFinding({
    challenge_report: JSON.stringify({ name: "webui-findings-challenger",
      base_sha: BASE, head_sha: HEAD,
      findings: [{ verdict: "reject", evidence: "No introduced defect." }] }),
  })), target), /confirmed findings/);
  assert.throws(() => readDecision(output(confirmedFinding({
    comments_json: JSON.stringify([{ path: "lib.rs", line: 10,
      body: "Finding not checked by challenger." }]),
  })), target), /matching challenger assessment/);
  assert.throws(() => readDecision(output(confirmedFinding({
    challenge_report: undefined,
  })), target), /bounded independent challenger report/);
});

test("only a complete, verified Mohamed PR can receive a native approval", () => {
  const item = readDecision(output(), eventTarget("pull_request_target", event()));
  assert.deepEqual(chooseReview(item, snapshot()).event, "APPROVE");
  assert.equal(chooseReview(item, snapshot("someone-else")).event, "COMMENT");
  assert.equal(chooseReview(item, snapshot("mddinbox")).event, null);
  assert.equal(chooseReview(item, snapshot("mohamedmansour",
    { checks: checks("failure") })).event, null);
  assert.equal(chooseReview(item, snapshot("mohamedmansour",
    { discussionComplete: false })).event, null);
  assert.equal(chooseReview(item, snapshot("mohamedmansour",
    { threads: [{ isResolved: false,
      comments: [{ author: "mddinbox" }] }] })).event, null);
  assert.equal(chooseReview(item, snapshot("mohamedmansour",
    { threads: [{ isResolved: false,
      comments: [{ author: "another-reviewer" }] }] })).event, null);
  assert.throws(() => chooseReview(item, snapshot("mohamedmansour",
    { pr: { ...pr(), base: { sha: "c".repeat(40),
      repo: { full_name: REPO } } } })), /revision changed/);
});

test("an earlier human COMMENT is not equivalent to a human APPROVE", () => {
      const item = readDecision(output(), eventTarget("pull_request_target", event()));
      const human = { user: { login: "mddinbox" }, commit_id: HEAD,
        state: "COMMENTED", body: "Prior note." };
      assert.equal(chooseReview(item, snapshot("mohamedmansour",
        { reviews: [human] })).event, "APPROVE");
      assert.equal(chooseReview(item, snapshot("mohamedmansour",
        { reviews: [{ ...human, state: "APPROVED" }] })).event, null);
});

test("confirmed findings on a human-authored PR may be COMMENTED, never approved", () => {
      const target = eventTarget("pull_request_target", event("pull_request_target", "mddinbox"));
      const item = readDecision(output(confirmedFinding()), target);
      assert.equal(chooseReview(item, snapshot("mddinbox")).event, "COMMENT");
      const clean = readDecision(output(), target);
      assert.equal(chooseReview(clean, snapshot("mddinbox")).event, null);
});

test("a prior finding remains orange on an unchanged head, even if a model says clean", () => {
  const oldReview = { user: { login: "github-actions[bot]" }, commit_id: HEAD,
    state: "COMMENTED",
    body: `Old finding.\n\n<!-- webui-ai-review:${HEAD}:findings -->` };
  const item = readDecision(output(), eventTarget("pull_request_target", event()));
  const action = chooseReview(item, snapshot("mohamedmansour", {
    reviews: [oldReview] }));
  assert.equal(action.event, null);
  assert.equal(action.label, "AI - Changes Required");
  const inconclusive = readDecision(output(decision({
    outcome: "inconclusive", coverage_complete: false,
  })), eventTarget("pull_request_target", event()));
  assert.equal(chooseReview(inconclusive, snapshot("mohamedmansour",
    { reviews: [oldReview] })).label, "AI - Changes Required");
});

test("new confirmed findings are posted even after an older bot approval", async () => {
  const previous = { user: { login: "github-actions[bot]" }, commit_id: HEAD,
    state: "APPROVED", id: 17,
    body: `Approved.\n\n<!-- webui-ai-review:${HEAD}:approved -->` };
  const client = mockClient({ currentReviews: [previous] });
  const item = confirmedFinding({ body: "A newly verified defect with a concrete fix." });
  await processReview("pull_request_target", event(), output(item), client, false);
  const review = JSON.parse(writes(client)[0].body);
  assert.equal(review.event, "COMMENT");
  assert.match(review.body, /earlier automated approval.*dismissal before merge/);
  assert.deepEqual(JSON.parse(writes(client)[1].body).labels,
    ["AI - Changes Required"]);
});

test("unchanged approval restores its label only while checks and discussion stay sound", () => {
  const target = eventTarget("pull_request_target", event());
  const unchanged = readDecision(output(decision({ outcome: "unchanged" })), target);
  const prior = { user: { login: "github-actions[bot]" }, commit_id: HEAD,
    state: "APPROVED", submitted_at: "2026-09-24T10:00:00Z",
    body: `Approved.\n\n<!-- webui-ai-review:${HEAD}:approved -->` };
  assert.equal(chooseReview(unchanged, snapshot("mohamedmansour",
    { reviews: [prior] })).label, "AI - Approved");
  assert.equal(chooseReview(unchanged, snapshot("mohamedmansour",
    { reviews: [prior], checks: checks("failure") })).label, null);
  assert.equal(chooseReview(unchanged, snapshot("mohamedmansour",
    { reviews: [prior, { user: { login: "reviewer" }, commit_id: HEAD,
      state: "CHANGES_REQUESTED", body: "Blocking issue.",
      submitted_at: "2026-09-24T11:00:00Z" }] })).label, null);
  assert.equal(chooseReview(unchanged, snapshot("mohamedmansour", {
    reviews: [prior, { user: { login: "reviewer" }, commit_id: HEAD,
      state: "COMMENTED", body: "New concern.",
      submitted_at: "2026-09-24T11:00:00Z" }],
  })).label, null);
  assert.equal(chooseReview(unchanged, snapshot("mohamedmansour", {
    reviews: [{ user: { login: "reviewer" }, commit_id: HEAD,
      state: "CHANGES_REQUESTED", body: "Earlier blocking finding.",
      submitted_at: "2026-09-24T09:00:00Z" }, prior],
  })).label, null);
  assert.equal(chooseReview(unchanged, snapshot("mohamedmansour", {
    reviews: [prior], threads: [{ isResolved: false,
      comments: [{ author: "another-reviewer", updatedAt: "2026-09-24T09:00:00Z" }] }],
  })).label, null);
  assert.equal(chooseReview(unchanged, snapshot("mohamedmansour",
    { reviews: [prior], issueComments: [{
      created_at: "2026-09-24T11:00:00Z", body: "New concern.",
    }] })).label, null);
});

test("staged start and publisher make no GitHub writes", async () => {
  const client = mockClient({ existingLabels: ["AI - Approved"] });
  assert.equal(await phaseReview("pull_request_target", event(), client, true, "start"), true);
  assert.match(await processReview("pull_request_target", event(), output(), client, true),
    /STAGED/);
  assert.equal(writes(client).length, 0);
});

test("a new head clears obsolete approval label before review", async () => {
  const client = mockClient({ existingLabels: ["AI - Approved"] });
  assert.equal(await phaseReview("pull_request_target", event(), client, false, "start"),
    true);
  assert.deepEqual(writes(client).map((request) => request.method), ["DELETE"]);
  assert.match(writes(client)[0].path, /AI%20-%20Approved/);
});

test("a failed publication clears AI labels; a successful one leaves them", async () => {
  const failed = mockClient({ existingLabels: ["AI - Changes Required"] });
  await phaseReview("pull_request_target", event(), failed, false, "finish", "failure");
  assert.deepEqual(writes(failed).map((request) => request.method), ["DELETE"]);
  const passed = mockClient({ existingLabels: ["AI - Approved"] });
  await phaseReview("pull_request_target", event(), passed, false, "finish", "success");
  assert.equal(writes(passed).length, 0);
});

test("publisher posts a real approval and then the green label", async () => {
  const client = mockClient();
  await processReview("pull_request_target", event(), output(), client, false);
  assert.deepEqual(writes(client).map((request) => request.method),
    ["POST", "POST"]);
  const review = JSON.parse(writes(client)[0].body);
  assert.equal(review.event, "APPROVE");
  assert.equal(review.commit_id, HEAD);
  assert.deepEqual(JSON.parse(writes(client)[1].body).labels, ["AI - Approved"]);
});

test("other authors only get LGTM COMMENT without Approved label", async () => {
  const client = mockClient({ author: "someone-else" });
  await processReview("pull_request_target", event("pull_request_target", "someone-else"),
    output(), client, false);
  assert.equal(writes(client).length, 1);
  const review = JSON.parse(writes(client)[0].body);
  assert.equal(review.event, "COMMENT");
  assert.match(review.body, /^LGTM/);
});

test("confirmed finding posts COMMENT before the orange label", async () => {
  const client = mockClient({ currentChecks: checks("failure") });
  const item = confirmedFinding({ body: "Trigger, impact, and smallest fix." });
  await processReview("pull_request_target", event(), output(item), client, false);
  assert.equal(JSON.parse(writes(client)[0].body).event, "COMMENT");
  assert.deepEqual(JSON.parse(writes(client)[1].body).labels,
    ["AI - Changes Required"]);
});

test("invalid inline anchors become actionable body findings", async () => {
  const client = mockClient();
  const item = confirmedFinding({ body: "One supported finding.",
    comments_json: JSON.stringify([
      { path: "DESIGN.md", line: 10, body: "Valid changed line." },
      { path: "DESIGN.md", line: 42, body: "Unanchorable finding and fix." },
    ]) });
  await processReview("pull_request_target", event(), output(item), client, false);
  const review = JSON.parse(writes(client)[0].body);
  assert.equal(review.comments.length, 1);
  assert.equal(review.comments[0].line, 10);
  assert.match(review.body, /DESIGN\.md:42: Unanchorable finding and fix/);
  const preview = anchorFindings({ event: "COMMENT",
    body: `Finding\n\n<!-- webui-ai-review:${HEAD}:findings -->`,
    comments: [{ path: "missing.rs", line: 1, body: "Root-cause fix." }] },
  [], HEAD);
  assert.equal(preview.comments.length, 0);
  assert.match(preview.body, /missing\.rs:1: Root-cause fix/);
});

test("a mismatched publication receipt cannot attach a label", async () => {
  const client = mockClient({ postedState: "COMMENTED" });
  await assert.rejects(processReview("pull_request_target", event(), output(),
    client, false), /receipt did not match/);
  assert.equal(writes(client).length, 1);
  const missingId = mockClient({ receiptId: null });
  await assert.rejects(processReview("pull_request_target", event(), output(),
    missingId, false), /receipt did not match/);
  assert.equal(writes(missingId).length, 1);
});

test("outcome label is created in the correct color only after the review receipt", async () => {
  const client = mockClient({ labelMissing: true });
  await processReview("pull_request_target", event(), output(), client, false);
  const mutations = writes(client);
  assert.equal(mutations[0].path, "/pulls/42/reviews");
  assert.equal(mutations[1].path, "/labels");
  assert.equal(JSON.parse(mutations[1].body).color, "0e8a16");
  assert.equal(mutations[2].path, "/issues/42/labels");
});

test("a head changed before posting prevents any write", async () => {
  const client = mockClient({ changedHeadOnSecondRead: true });
  await assert.rejects(processReview("pull_request_target", event(), output(),
    client, false), /revision changed/);
  assert.equal(writes(client).length, 0);
});

test("a new PR comment before posting invalidates the reviewed discussion", async () => {
  const client = mockClient();
  let reads = 0;
  client.snapshot = async () => ({
    ...snapshot(),
    issueComments: ++reads === 1 ? [] : [{ id: 1, body: "New author response." }],
  });
  await assert.rejects(processReview("pull_request_target", event(), output(),
    client, false), /review decision changed/);
  assert.equal(writes(client).length, 0);
});

test("a new human concern after approval receipt prevents green labeling", async () => {
  const client = mockClient();
  const read = client.snapshot.bind(client);
  let reads = 0;
  client.snapshot = async (...args) => {
    const current = await read(...args);
    reads++;
    if (reads > 2) current.issueComments = [{
      created_at: "2026-09-24T11:01:00Z",
      updated_at: "2026-09-24T11:01:00Z", body: "Concern after review.",
    }];
    return current;
  };
  await assert.rejects(processReview("pull_request_target", event(), output(),
    client, false), /changed before labeling/);
  assert.deepEqual(writes(client).map(({ path }) => path), ["/pulls/42/reviews"]);
});

test("pending CI polls until required checks succeed", async () => {
  const pending = { name: "PR Checks", status: "in_progress", conclusion: null };
  const client = mockClient();
  const states = [pending, pending, checks()[0]];
  client.snapshot = async () => ({ ...snapshot(), checks: [states.shift()] });
  let waits = 0;
  const item = readDecision(output(), eventTarget("pull_request_target", event()));
  const result = await waitForChecks(client, item, async () => { waits++; }, 3);
  assert.equal(result.checks[0].conclusion, "success");
  assert.equal(waits, 2);
  assert.equal(writes(client).length, 0);
});

test("a staged clean decision exercises read-only CI polling", async () => {
  const client = mockClient();
  let polls = 0;
  const preview = await processReview("pull_request_target", event(), output(),
    client, true, async (_client, _decision, _pause, _attempts, initial) => {
      polls++;
      assert.ok(initial);
      return initial;
    });
  assert.equal(polls, 1);
  assert.match(preview, /STAGED #42: APPROVE/);
  assert.equal(writes(client).length, 0);
});

test("staged run actually waits for pending PR Checks without publishing", async () => {
  const client = mockClient();
  let reads = 0;
  client.snapshot = async () => ({
    ...snapshot(), checks: [reads++ < 1 ?
      { name: "PR Checks", status: "in_progress", conclusion: null } :
      checks()[0]],
  });
  let polls = 0;
  const preview = await processReview("pull_request_target", event(), output(),
    client, true, (...args) => waitForChecks(...args.slice(0, 2),
      async () => { polls++; }, 3, args[4]));
  assert.equal(polls, 1);
  assert.match(preview, /STAGED #42: APPROVE/);
  assert.equal(writes(client).length, 0);
});

test("a bounded CI wait never converts missing checks to success", async () => {
  const client = mockClient();
  client.snapshot = async () => ({
    ...snapshot(), checks: [{ name: "PR Checks",
      status: "in_progress", conclusion: null }],
  });
  const item = readDecision(output(), eventTarget("pull_request_target", event()));
  const result = await waitForChecks(client, item, async () => {}, 2);
  assert.equal(chooseReview(item, result).event, null);
  assert.equal(writes(client).length, 0);
});

test("a newer queued PR Checks run cannot reuse an older success", () => {
  const item = readDecision(output(), eventTarget("pull_request_target", event()));
  const previous = checks()[0];
  const pending = { name: "PR Checks", status: "queued", conclusion: null,
    app: { slug: "github-actions" },
    created_at: "2026-09-24T11:00:00Z", started_at: null };
  assert.equal(chooseReview(item, snapshot("mohamedmansour",
    { checks: [previous, pending] })).event, null);
});

test("a similarly named check from another app cannot authorize approval", () => {
  const item = readDecision(output(), eventTarget("pull_request_target", event()));
  const untrusted = { ...checks()[0], app: { slug: "other-app" } };
  assert.equal(chooseReview(item, snapshot("mohamedmansour",
    { checks: [untrusted] })).event, null);
});

test("GitHub API errors fail explicitly without a success-shaped response", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => ({ ok: false, status: 403 });
  try {
    await assert.rejects(githubClient("test-token").call("/pulls/42"), /HTTP 403/);
  } finally {
    globalThis.fetch = originalFetch;
  }
});
