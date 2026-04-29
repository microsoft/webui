# Contact Book Benchmark

End-to-end performance benchmark for the WebUI framework, using the
**contact-book-manager** example application as a realistic workload.

## What it measures

| Benchmark Group | What it does |
|---|---|
| **`contact_book_protocol_parse`** | Deserializes the compiled protocol binary (`WebUIProtocol::from_protobuf`) — measures the cost of loading a protocol at startup. |
| **`contact_book_render`** | Renders the full contact-book dashboard (protocol + state → HTML) without any hydration plugin, at 10 / 100 / 1,000 contacts. |
| **`contact_book_render_fast_plugin`** | Same rendering with the deprecated FAST 2 compatibility plugin enabled, which injects legacy FAST hydration markers. |

### Why it stays up to date

The protocol is **compiled from live source** at benchmark time via
`webui::build()` against `examples/app/contact-book-manager/src/`. There is no
cached binary — any change to the contact-book-manager templates is
automatically reflected in the next benchmark run.

## Running the benchmark

### Quick validation (no measurements)

```bash
cargo bench -p webui --bench contact_book_bench -- --test
```

Compiles in release mode and runs each benchmark once to verify correctness.
Takes ~1 minute (mostly compile time).

### Full benchmark

```bash
cargo bench -p webui --bench contact_book_bench
```

Runs all benchmark groups with 30-second measurement windows. Produces:

1. **Criterion output** — per-benchmark timing, throughput (MiB/s), and change
   detection printed inline.
2. **Summary table** — a compact table printed at the end with Iters, Avg, Min,
   Max, Dev%, P50, P90, P99, IQR, and output Bytes for every scenario.
3. **HTML reports** — detailed charts saved to `target/criterion/report/index.html`.

### Run a single group

```bash
# Only protocol parsing
cargo bench -p webui --bench contact_book_bench -- "contact_book_protocol_parse"

# Only rendering at 100 contacts
cargo bench -p webui --bench contact_book_bench -- "contact_book_render/contacts/100"

# Only FAST 2 compatibility plugin benchmarks
cargo bench -p webui --bench contact_book_bench -- "contact_book_render_fast_plugin"
```

## Reading the results

### Inline output

Criterion prints results as each benchmark completes:

```
contact_book_render/contacts/100
                        time:   [5.05 ms 5.09 ms 5.12 ms]
                        thrpt:  [10.5 MiB/s 10.6 MiB/s 10.6 MiB/s]
```

- **time** — [lower bound, estimate, upper bound] at 95% confidence.
- **thrpt** — throughput in MiB/s based on HTML output size.

### Summary table

Printed at the end of a full run:

```
===================== WebUI Contact Book — Performance Summary =====================
Story                  Iters   Avg(ms)     Min       Max   Dev%     P50     P90     P99     IQR   Bytes
-------------------------------------------------------------------------------------
ProtocolParse          55000      0.05    0.04      0.37  12.0%    0.05    0.05    0.08    0.00   28538
Render/10               4600      0.65    0.61     10.34  28.2%    0.63    0.66    1.22    0.02   25960
Render/100               600      4.94    4.70      9.03   9.4%    4.80    5.21    7.43    0.11   56397
Render/1000               53     57.50   53.78     67.33   4.6%   57.20   60.90   62.28    4.31  362930
RenderFAST/10           4600      0.65    0.61      1.83  13.7%    0.63    0.66    1.19    0.02   31052
RenderFAST/100           600      5.02    4.72      9.86  14.1%    4.81    5.26    9.09    0.11   68149
RenderFAST/1000           51     59.53   53.19     72.35   7.2%   58.64   64.56   72.35    4.83  443082
=====================================================================================
```

| Column | Meaning |
|---|---|
| **Iters** | Total iterations completed during the sampling window. |
| **Avg(ms)** | Mean time per iteration. |
| **Min / Max** | Fastest and slowest observed iteration. |
| **Dev%** | Standard deviation as a percentage of the mean. |
| **P50 / P90 / P99** | Percentile latencies (P50 = median). |
| **IQR** | Interquartile range (P75 − P25) — lower means more consistent. |
| **Bytes** | Output size in bytes (protocol size for parse, HTML size for render). |

## Detecting regressions and improvements

### Automatic change detection

When you run the benchmark a second time, criterion compares against the
previous baseline and reports the delta:

```
contact_book_render/contacts/100
                        time:   [5.05 ms 5.09 ms 5.12 ms]
                 change:
                        time:   [+2.60% +3.37% +4.20%] (p = 0.00 < 0.05)
                        Performance has regressed.
```

- **Performance has improved** — the change is statistically significant and
  faster.
- **Performance has regressed** — the change is statistically significant and
  slower.
- **No change in performance** — the difference is within noise.

### HTML reports

Open `target/criterion/report/index.html` in a browser. Each benchmark has:

- **PDF/CDF plots** of iteration times.
- **Before/after violin plots** when a baseline exists.
- **Regression analysis** with confidence intervals.

### Tips for reliable measurements

- **Close other applications** — CPU-intensive background work adds noise.
- **Run on the same machine** — cross-machine comparisons are not meaningful.
- **Use release mode** — `cargo bench` always compiles with optimizations;
  debug builds are not representative.
- **Compare P50 over Avg** — the median is more robust to outliers than the
  mean, especially on machines with thermal throttling or background activity.
- **Watch IQR and Dev%** — high values indicate noisy measurements. Re-run if
  Dev% exceeds ~15% for the larger benchmarks.

### Resetting the baseline

To discard previous results and start fresh:

```bash
rm -rf target/criterion
cargo bench -p webui --bench contact_book_bench
```
