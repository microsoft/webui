---
title: Benchmark results
description: Inspect the uniform official WebUI benchmark campaign across JavaScript heap, renderer memory, LCP, and server throughput, with 15 cases and explicit provenance limits.
layout: page
outline: false
stateFile: ./.webui-press/generated/benchmark-state.json
---

# Measurements, with their limits

Different measurements answer different questions. Start with browser memory,
then inspect paint timing or server throughput. These results are from the
uniform official campaign and are not a current framework ranking or an overall
performance score. This page presents all 15 configurations in the campaign.

The server snapshot contains **one 20-second measured round per response mode**.
Browser metrics describe **10 isolated runs per configuration**.

<benchmark-explorer :benchmarks="{{benchmarks}}"></benchmark-explorer>

## What this snapshot describes

The recorded server workload uses 100 todos, 13 component types, 32 warm
structural variants, and current UTC state materialized per request. It records
64 connections from two load workers against one server process/worker pinned
to one CPU, with one 20-second measured round. Responses retain their natural
byte sizes.

The browser setup identifies Chromium 156.0.8078.4, a 1,440 × 900 viewport,
10 randomized cold-cache navigations, and a fresh foreground
process/context/page per sample. Native occlusion throttling was disabled;
there was no CPU or network throttling. These are lab measurements, not
field Core Web Vitals.

- **JavaScript heap** is recorded after readiness and explicit garbage
  collection. It is only part of browser memory.
- **Renderer private bytes** describe the isolated Chromium renderer process,
  not server RSS. Both memory metrics display MiB, where 1 MiB is 1,048,576 bytes;
  the download retains the original byte values. Some renderer-private samples
  are unavailable; the per-case notes disclose the sample counts.
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

The [benchmark source](https://github.com/microsoft/webui-benchmarks) records
the complete and progressive SSR reports and the progressive browser report.
The displayed campaign uses released WebUI 3.0.4 and one clean source revision
across all 15 cases; it is not the earlier selective refresh.

The sanitized extract does not embed the complete raw-run archive or every
framework binary artifact. Campaign source revision, fixture digest, WebUI
version, response modes, sampling counts, and methodology are recorded in the
benchmark evidence contract.

One server round cannot estimate run-to-run variability. A MAD of zero in that
record is not a confidence claim. Browser MAD and p95 notes remain descriptive;
without repeated independent server rounds, small differences
must not be promoted as statistically established wins.

## Inspect the data, then measure your application

[Download the displayed data as JSON](/benchmarks/official-results.json).
This generated, sanitized extract includes the 15 displayed cases, the five metrics,
recorded notes, sampling information, and the limitations on this page.
It omits bulk server telemetry and unsupported winner summaries. It makes the
display inspectable; it is **not a complete reproducibility package**.

Use these results to frame questions, not to choose a framework from a single
number. Response size, memory, startup, interaction, and sustained load each need
measurement in your own environment. The [performance guide](/guide/concepts/performance/)
explains where WebUI moves work and how to evaluate those tradeoffs.
