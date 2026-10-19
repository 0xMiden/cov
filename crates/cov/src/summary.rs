//! Human-readable coverage summary for the terminal.

use std::io::{self, Write};

use crate::{Report, source::display_relative};

/// Writes a per-file table of line, function, and branch coverage with a
/// total row, followed by the stored paths that could not be resolved.
pub fn write(report: &Report, out: &mut dyn Write) -> io::Result<()> {
    let mut rows: Vec<(String, Counts)> = report
        .files
        .values()
        .map(|file| {
            let counts = Counts {
                lines: Ratio::of(file.lines.values().copied()),
                functions: Ratio::of(file.functions.iter().map(|f| f.count)),
                branches: Ratio::of(file.branches.iter().flat_map(|b| b.arms)),
            };
            (display_relative(&file.path), counts)
        })
        .collect();
    let total = rows.iter().fold(Counts::default(), |total, (_, counts)| Counts {
        lines: total.lines.add(counts.lines),
        functions: total.functions.add(counts.functions),
        branches: total.branches.add(counts.branches),
    });
    rows.push(("TOTAL".into(), total));

    let header = ["File", "Lines", "Functions", "Branches"];
    let cells: Vec<[String; 4]> = rows
        .into_iter()
        .map(|(name, counts)| {
            [
                name,
                counts.lines.to_string(),
                counts.functions.to_string(),
                counts.branches.to_string(),
            ]
        })
        .collect();
    let mut widths = header.map(str::len);
    for row in &cells {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.len());
        }
    }

    let [file_w, lines_w, functions_w, branches_w] = widths;
    let rule = "-".repeat(file_w + lines_w + functions_w + branches_w + 6);
    let [h_file, h_lines, h_functions, h_branches] = header;
    writeln!(
        out,
        "{h_file:<file_w$}  {h_lines:>lines_w$}  {h_functions:>functions_w$}  {h_branches:>branches_w$}"
    )?;
    writeln!(out, "{rule}")?;
    let last = cells.len() - 1;
    for (i, [file, lines, functions, branches]) in cells.iter().enumerate() {
        if i == last {
            writeln!(out, "{rule}")?;
        }
        writeln!(
            out,
            "{file:<file_w$}  {lines:>lines_w$}  {functions:>functions_w$}  {branches:>branches_w$}"
        )?;
    }

    if !report.unresolved.is_empty() {
        writeln!(out)?;
        writeln!(out, "Unresolved sources:")?;
        for path in &report.unresolved {
            writeln!(out, "  {path}")?;
        }
    }
    Ok(())
}

/// Hit and total counts of one file or of the whole report.
#[derive(Debug, Clone, Copy, Default)]
struct Counts {
    lines: Ratio,
    functions: Ratio,
    branches: Ratio,
}

/// How many of `total` items were hit.
#[derive(Debug, Clone, Copy, Default)]
struct Ratio {
    hit: usize,
    total: usize,
}

impl Ratio {
    /// Counts the non-zero entries of `counts`.
    fn of(counts: impl Iterator<Item = u64>) -> Self {
        counts.fold(Self::default(), |ratio, count| Self {
            hit: ratio.hit + usize::from(count > 0),
            total: ratio.total + 1,
        })
    }

    fn add(self, other: Self) -> Self {
        Self {
            hit: self.hit + other.hit,
            total: self.total + other.total,
        }
    }
}

impl core::fmt::Display for Ratio {
    /// `hit/total  pct%`, or `-` when there is nothing to cover.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.total == 0 {
            return write!(f, "-");
        }
        let percent = 100.0 * self.hit as f64 / self.total as f64;
        write!(f, "{}/{} {percent:5.1}%", self.hit, self.total)
    }
}
