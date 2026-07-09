// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Generic baseline snapshots for regression gating.
//!
//! Every resource bench can `--save <name>` a JSON snapshot of its results and
//! later `--compare <name>` against it. The snapshot format, on-disk location
//! (`target/bench-baselines/<kind>-<name>.json`), schema check and Δ% diffing
//! are all handled here, generically over the bench's own row type.
//!
//! A bench row implements [`BaselineRow`] to expose a stable `key` (for matching
//! current vs baseline rows) and the numeric [`Metric`]s to diff. The row type
//! also derives `serde::{Serialize, Deserialize}` for storage.
//!
//! ```ignore
//! #[derive(serde::Serialize, serde::Deserialize)]
//! struct Row { label: String, user_us: f64, ops_per_s: f64 }
//!
//! impl BaselineRow for Row {
//!     fn key(&self) -> String { self.label.clone() }
//!     fn metrics(&self) -> Vec<Metric> {
//!         vec![
//!             Metric::lower_better("user µs", self.user_us),
//!             Metric::higher_better("ops/s", self.ops_per_s),
//!         ]
//!     }
//! }
//! ```

use crate::report::Table;
use console::style;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::PathBuf;

/// A single numeric metric on a baseline row, plus its "good direction".
#[derive(Clone, Copy, Debug)]
pub struct Metric {
    /// Column label shown in the diff table.
    pub name: &'static str,
    /// The value to compare.
    pub value: f64,
    /// When `true`, a *lower* value is better (cpu, bytes, latency). When
    /// `false`, a *higher* value is better (throughput).
    pub lower_is_better: bool,
}

impl Metric {
    /// A metric where lower is better (cpu µs, bytes, latency).
    #[must_use]
    pub fn lower_better(name: &'static str, value: f64) -> Self {
        Self {
            name,
            value,
            lower_is_better: true,
        }
    }

    /// A metric where higher is better (ops/s, MiB/s throughput).
    #[must_use]
    pub fn higher_better(name: &'static str, value: f64) -> Self {
        Self {
            name,
            value,
            lower_is_better: false,
        }
    }
}

/// A bench result row that can be stored in and diffed against a baseline.
pub trait BaselineRow {
    /// Stable identity used to match a current row to a baseline row
    /// (e.g. `"parse @ 1000"`).
    fn key(&self) -> String;
    /// The numeric metrics to diff, in display order. All rows of a bench must
    /// return the same metric names in the same order.
    fn metrics(&self) -> Vec<Metric>;
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot<R> {
    schema: u32,
    kind: String,
    name: String,
    timestamp_unix: u64,
    rows: Vec<R>,
}

fn snapshot_path(kind: &str, name: &str) -> PathBuf {
    // This crate lives at `crates/webui-bench-support`; up two is the workspace
    // root, matching the `target/bench-baselines` convention shared by all
    // benches regardless of which crate's example is running.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("bench-baselines")
        .join(format!("{kind}-{name}.json"))
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Save `rows` as the baseline `<kind>-<name>.json`. Prints the path on success
/// and a diagnostic on failure (never panics).
pub fn save<R>(kind: &str, name: &str, schema: u32, rows: Vec<R>)
where
    R: Serialize,
{
    let path = snapshot_path(kind, name);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let snap = Snapshot {
        schema,
        kind: kind.to_string(),
        name: name.to_string(),
        timestamp_unix: now_unix(),
        rows,
    };
    match serde_json::to_string_pretty(&snap) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!("snapshot: write {} failed: {e}", path.display());
                return;
            }
            println!();
            println!(
                "{} Baseline saved to {}",
                style("✔").green().bold(),
                style(path.display()).bold()
            );
        }
        Err(e) => eprintln!("snapshot: serialize failed: {e}"),
    }
}

/// Load and schema-check the baseline `<kind>-<name>.json`.
///
/// Returns `None` (with a diagnostic) if the file is missing, unparsable, or
/// carries a different `schema`.
fn load<R>(kind: &str, name: &str, schema: u32) -> Option<(Vec<R>, u64)>
where
    R: DeserializeOwned,
{
    let path = snapshot_path(kind, name);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!(
                "compare: baseline '{name}' not found at {} — run with --save {name} first",
                path.display()
            );
            return None;
        }
    };
    match serde_json::from_slice::<Snapshot<R>>(&bytes) {
        Ok(s) if s.schema == schema => Some((s.rows, s.timestamp_unix)),
        Ok(s) => {
            eprintln!(
                "compare: baseline '{name}' has schema {} (expected {schema}); regenerate with --save",
                s.schema
            );
            None
        }
        Err(e) => {
            eprintln!("compare: parse {} failed: {e}", path.display());
            None
        }
    }
}

fn pct_change(base: f64, current: f64) -> f64 {
    if base == 0.0 {
        return 0.0;
    }
    ((current - base) / base) * 100.0
}

/// Whether a Δ% is a regression in the metric's "good direction" beyond
/// `threshold` (a positive percentage).
fn is_regression(delta_pct: f64, lower_is_better: bool, threshold: f64) -> bool {
    if lower_is_better {
        delta_pct > threshold
    } else {
        delta_pct < -threshold
    }
}

/// Compare `current` rows against baseline `<kind>-<name>`, printing a Δ% table
/// per metric and a pass/fail summary against `threshold_pct`.
///
/// Returns `true` when every metric is within threshold (or the baseline is
/// missing, in which case there is nothing to fail against).
pub fn compare<R>(kind: &str, name: &str, schema: u32, current: &[R], threshold_pct: f64) -> bool
where
    R: BaselineRow + DeserializeOwned,
{
    let Some((base_rows, timestamp)) = load::<R>(kind, name, schema) else {
        return true;
    };
    let base_index: Vec<(String, Vec<Metric>)> =
        base_rows.iter().map(|r| (r.key(), r.metrics())).collect();

    // Header: row key + one Δ% column per metric.
    let metric_names: Vec<&'static str> = current
        .first()
        .map(|r| r.metrics().into_iter().map(|m| m.name).collect())
        .unwrap_or_default();
    let mut headers: Vec<String> = Vec::with_capacity(metric_names.len() + 1);
    headers.push("row".to_string());
    for n in &metric_names {
        headers.push(format!("{n} Δ%"));
    }
    let mut aligns = vec![crate::report::Align::Left];
    aligns.extend(std::iter::repeat_n(
        crate::report::Align::Right,
        metric_names.len(),
    ));
    let mut table = Table::new(headers).aligns(aligns);

    let mut regressions: Vec<String> = Vec::new();
    for cur in current {
        let key = cur.key();
        let cur_metrics = cur.metrics();
        let base_metrics = base_index.iter().find(|(k, _)| *k == key).map(|(_, m)| m);
        let mut cells: Vec<String> = Vec::with_capacity(cur_metrics.len() + 1);
        cells.push(key.clone());
        match base_metrics {
            Some(base_metrics) => {
                for cm in &cur_metrics {
                    let base_val = base_metrics
                        .iter()
                        .find(|bm| bm.name == cm.name)
                        .map(|bm| bm.value);
                    match base_val {
                        Some(bv) => {
                            let d = pct_change(bv, cm.value);
                            cells.push(format!("{d:+.1}%"));
                            if is_regression(d, cm.lower_is_better, threshold_pct) {
                                regressions.push(format!("{key} · {} {d:+.1}%", cm.name));
                            }
                        }
                        None => cells.push("—".to_string()),
                    }
                }
            }
            None => {
                for _ in &cur_metrics {
                    cells.push("(new)".to_string());
                }
            }
        }
        table.row(cells);
    }

    println!();
    println!(
        "Diff vs baseline '{}' (saved {} ago)",
        style(name).bold(),
        format_age(timestamp)
    );
    table.print();
    println!();

    if regressions.is_empty() {
        println!(
            "{} all metrics within ±{threshold_pct:.0}% of baseline.",
            style("✔").green().bold()
        );
        true
    } else {
        println!(
            "{} {} metric(s) regressed beyond ±{threshold_pct:.0}%:",
            style("✘").red().bold(),
            regressions.len()
        );
        for r in &regressions {
            println!("  {} {}", style("•").red(), r);
        }
        false
    }
}

fn format_age(timestamp_unix: u64) -> String {
    let now = now_unix();
    let secs = now.saturating_sub(timestamp_unix);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pct_change_handles_zero_base() {
        assert_eq!(pct_change(0.0, 5.0), 0.0);
        assert!((pct_change(100.0, 110.0) - 10.0).abs() < 1e-9);
        assert!((pct_change(100.0, 90.0) + 10.0).abs() < 1e-9);
    }

    #[test]
    fn regression_direction_depends_on_metric() {
        // lower-is-better: +10% over a 5% threshold is a regression.
        assert!(is_regression(10.0, true, 5.0));
        assert!(!is_regression(-10.0, true, 5.0));
        // higher-is-better: -10% (throughput dropped) is a regression.
        assert!(is_regression(-10.0, false, 5.0));
        assert!(!is_regression(10.0, false, 5.0));
        // Within threshold either way: not a regression.
        assert!(!is_regression(3.0, true, 5.0));
        assert!(!is_regression(-3.0, false, 5.0));
    }
}
