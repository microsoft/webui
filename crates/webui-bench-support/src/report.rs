// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Consistent, console-styled result tables and numeric formatters.
//!
//! Every resource bench prints its per-scale results through a [`Table`] so the
//! column layout, alignment and styling are identical across benches. Cell
//! contents are formatted by the bench (using [`format_bytes`], [`format_ops`],
//! [`format_count`]); the table owns width computation, alignment and the
//! `console`-styled header/separator.

use console::style;

/// Column alignment.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Align {
    /// Left-align (labels).
    Left,
    /// Right-align (numbers).
    Right,
}

/// A simple fixed-width results table with a styled header.
///
/// Columns default to right-aligned (numeric); set per-column alignment with
/// [`Table::aligns`]. Widths are computed from the widest cell so the output
/// stays aligned regardless of value magnitude.
pub struct Table {
    headers: Vec<String>,
    aligns: Vec<Align>,
    rows: Vec<Vec<String>>,
}

impl Table {
    /// Create a table with the given column headers (all right-aligned).
    #[must_use]
    pub fn new<I, S>(headers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let headers: Vec<String> = headers.into_iter().map(Into::into).collect();
        let aligns = vec![Align::Right; headers.len()];
        Self {
            headers,
            aligns,
            rows: Vec::new(),
        }
    }

    /// Override per-column alignment. Extra entries are ignored; missing ones
    /// stay right-aligned.
    #[must_use]
    pub fn aligns<I>(mut self, aligns: I) -> Self
    where
        I: IntoIterator<Item = Align>,
    {
        for (slot, a) in self.aligns.iter_mut().zip(aligns) {
            *slot = a;
        }
        self
    }

    /// Append a row. Cells are used positionally against the headers.
    pub fn row<I, S>(&mut self, cells: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.rows.push(cells.into_iter().map(Into::into).collect());
        self
    }

    /// Compute the display width of each column.
    fn widths(&self) -> Vec<usize> {
        let mut widths: Vec<usize> = self.headers.iter().map(|h| h.chars().count()).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < widths.len() {
                    widths[i] = widths[i].max(cell.chars().count());
                }
            }
        }
        widths
    }

    fn pad(cell: &str, width: usize, align: Align) -> String {
        let len = cell.chars().count();
        if len >= width {
            return cell.to_string();
        }
        let gap = " ".repeat(width - len);
        match align {
            Align::Left => format!("{cell}{gap}"),
            Align::Right => format!("{gap}{cell}"),
        }
    }

    fn align_of(&self, i: usize) -> Align {
        self.aligns.get(i).copied().unwrap_or(Align::Right)
    }

    /// Render the table to stdout with a styled header and separator.
    pub fn print(&self) {
        let widths = self.widths();

        let header_cells: Vec<String> = self
            .headers
            .iter()
            .enumerate()
            .map(|(i, h)| Self::pad(h, widths[i], self.align_of(i)))
            .collect();
        println!("| {} |", style(header_cells.join(" | ")).cyan().bold());

        let sep: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
        println!("| {} |", style(sep.join(" | ")).dim());

        for row in &self.rows {
            let cells: Vec<String> = (0..self.headers.len())
                .map(|i| {
                    let cell = row.get(i).map(String::as_str).unwrap_or("");
                    Self::pad(cell, widths[i], self.align_of(i))
                })
                .collect();
            println!("| {} |", cells.join(" | "));
        }
    }
}

/// Format a per-op byte count as `B` / `KiB` / `MiB`.
#[must_use]
pub fn format_bytes(bytes: f64) -> String {
    if bytes < 1024.0 {
        format!("{bytes:.0} B")
    } else if bytes < 1024.0 * 1024.0 {
        format!("{:.1} KiB", bytes / 1024.0)
    } else {
        format!("{:.2} MiB", bytes / (1024.0 * 1024.0))
    }
}

/// Format an operations/second rate compactly (`k` / `M` suffixes).
#[must_use]
pub fn format_ops(ops: f64) -> String {
    if ops >= 1_000_000.0 {
        format!("{:.1}M", ops / 1_000_000.0)
    } else if ops >= 1_000.0 {
        format!("{:.0}k", ops / 1_000.0)
    } else {
        format!("{ops:.0}")
    }
}

/// Format a large count with `k` / `M` suffixes for readability.
#[must_use]
pub fn format_count(count: f64) -> String {
    if count >= 1_000_000.0 {
        format!("{:.2}M", count / 1_000_000.0)
    } else if count >= 10_000.0 {
        format!("{:.1}k", count / 1_000.0)
    } else {
        format!("{count:.0}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_scales() {
        assert_eq!(format_bytes(512.0), "512 B");
        assert_eq!(format_bytes(2048.0), "2.0 KiB");
        assert_eq!(format_bytes(5.0 * 1024.0 * 1024.0), "5.00 MiB");
    }

    #[test]
    fn format_ops_scales() {
        assert_eq!(format_ops(42.0), "42");
        assert_eq!(format_ops(318_000.0), "318k");
        assert_eq!(format_ops(1_500_000.0), "1.5M");
    }

    #[test]
    fn pad_respects_alignment() {
        assert_eq!(Table::pad("ab", 5, Align::Right), "   ab");
        assert_eq!(Table::pad("ab", 5, Align::Left), "ab   ");
        // No truncation when the cell is already wide enough.
        assert_eq!(Table::pad("abcdef", 3, Align::Right), "abcdef");
    }

    #[test]
    fn widths_track_widest_cell() {
        let mut t = Table::new(["a", "bb"]);
        t.row(["xxxx", "y"]);
        assert_eq!(t.widths(), vec![4, 2]);
    }
}
