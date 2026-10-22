---
title: Benchmark results
description: Inspect WebUI's historical benchmark results across JavaScript heap, renderer memory, LCP, and server throughput, with 14 selected cases and explicit provenance limits.
layout: page
outline: false
stateFile: ./.webui-press/generated/benchmark-state.json
---

# Measurements, with their limits

Different measurements answer different questions. Start with browser memory,
then inspect paint timing or server throughput. These are **historical, limited
results**, not a current framework ranking or an overall performance score.
This page presents 14 selected configurations from the recorded snapshot.

The server snapshot contains **one measured round**. Browser metrics describe
**10 isolated runs**. Three WebUI hosts were selectively refreshed on
**2026-09-04**; the other 11 cases retain earlier official results. This is not
a uniform rerun.

<benchmark-explorer :benchmarks="{{benchmarks}}"></benchmark-explorer>

## What this snapshot describes

The recorded server workload uses 100 todos, 13 component types, 32 warm
structural variants, and current UTC state materialized per request. It records
64 connections from two load workers against one server process/worker pinned
to one CPU, with one 20-second measured round. Responses retain their natural
byte sizes.

The browser setup identifies Chromium 151.0.7922.34, a 1,440 × 900 viewport,
10 randomized cold-cache navigations, and a fresh foreground
process/context/page per sample. Native occlusion throttling was disabled;
there was no CPU or network throttling. These are lab measurements, not
field Core Web Vitals.

- **JavaScript heap** is recorded after readiness and explicit garbage
  collection. It is only part of browser memory.
- **Renderer private bytes** describe the isolated Chromium renderer process,
  not server RSS. Both memory metrics display MiB, where 1 MiB is 1,048,576 bytes;
  the download retains the original byte values.
- **LCP** is pre-interaction largest contentful paint from navigation start,
  in milliseconds. It does not describe interaction responsiveness.
- **Complete and progressive throughput** are requests per second under
  different response modes. The progressive category preserves each case's
  existing behavior, including buffered synchronous routes. A dedicated WebUI
  resumable async data-boundary workload was not measured.

Some server notes report still-rising 128-connection saturation probes.
The displayed measurements remain the 64-connection results, not demonstrated
maximum throughput. CPU/request and server RSS are different measurements;
they are not combined with these five metrics or presented as an overall score.

## What is missing

The advertised [benchmark source](https://github.com/microsoft/webui-benchmarks)
returned **HTTP 404 without authentication** during the website audit. We could
not verify a publicly inspectable source revision or the full raw artifacts.
We do not know whether that source is private, moved, or unavailable.

The available summary does not establish a complete per-case framework/runtime
version trail, the original measurement dates for the other cases, a complete
hardware/OS record, or an unambiguous browser response-mode record.
The mention of **WebUI 0.0.28** in capability metadata does **not** prove
that every measurement used that version.

Only the WebUI Light DOM **Bun.serve, Deno.serve, and Node node:http** cases were
refreshed on 2026-09-04. The diamond beside those labels records that provenance,
not a quality rating.

One server round cannot estimate run-to-run variability. A MAD of zero in that
record is not a confidence claim. Browser MAD and p95 notes remain descriptive;
without the raw samples and a consistent comparison campaign, small differences
must not be promoted as statistically established wins.

## Inspect the data, then measure your application

[Download the displayed data as JSON](/benchmarks/historical-results.json).
This generated, sanitized extract includes the 14 displayed cases, the five metrics,
recorded notes, sampling information, and the limitations on this page.
It omits bulk server telemetry and unsupported winner summaries. It makes the
display inspectable; it is **not a complete reproducibility package**.

Use these results to frame questions, not to choose a framework from a single
number. Response size, memory, startup, interaction, and sustained load each need
measurement in your own environment. The [performance guide](/guide/concepts/performance/)
explains where WebUI moves work and how to evaluate those tradeoffs.
