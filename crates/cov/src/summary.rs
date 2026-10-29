//! Human-readable coverage summary for the terminal.

use std::io::{self, Write};

use crate::{Report, compare::Totals, source::display_relative};

/// Writes a per-file table of line, function, and branch coverage with a
/// total row, followed by one line per package with stored paths that could
/// not be resolved.
pub fn write(report: &Report, out: &mut dyn Write) -> io::Result<()> {
    let mut rows: Vec<(String, Totals)> = report
        .files
        .values()
        .map(|file| (display_relative(&file.path), Totals::of(file)))
        .collect();
    rows.push(("TOTAL".into(), Totals::of_report(report)));

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
        for line in report.unresolved_summary() {
            writeln!(out, "  {line}")?;
        }
    }
    Ok(())
}
