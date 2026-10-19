//! LCOV tracefile output.
//!
//! The format is the one `geninfo` writes and `genhtml` reads; see the
//! `geninfo(1)` manual page.

use std::io::{self, Write};

use crate::Report;

/// Writes `report` as an LCOV tracefile, one record per file in path order.
pub fn write(report: &Report, out: &mut dyn Write) -> io::Result<()> {
    for file in report.files.values() {
        writeln!(out, "TN:")?;
        writeln!(out, "SF:{}", crate::source::absolute_path(&file.path).display())?;

        for function in &file.functions {
            writeln!(out, "FN:{},{}", function.line, function.name)?;
        }
        for function in &file.functions {
            writeln!(out, "FNDA:{},{}", function.count, function.name)?;
        }
        writeln!(out, "FNF:{}", file.functions.len())?;
        writeln!(out, "FNH:{}", file.functions.iter().filter(|f| f.count > 0).count())?;

        for branch in &file.branches {
            for (arm, count) in branch.arms.iter().enumerate() {
                // `-` marks a branch whose condition was never evaluated, as
                // opposed to `0` for an arm not taken when it was.
                if branch.executed {
                    writeln!(out, "BRDA:{},{},{arm},{count}", branch.line, branch.block)?;
                } else {
                    writeln!(out, "BRDA:{},{},{arm},-", branch.line, branch.block)?;
                }
            }
        }
        let arms_hit: usize = file
            .branches
            .iter()
            .map(|branch| branch.arms.iter().filter(|count| **count > 0).count())
            .sum();
        writeln!(out, "BRF:{}", 2 * file.branches.len())?;
        writeln!(out, "BRH:{arms_hit}")?;

        for (line, count) in &file.lines {
            writeln!(out, "DA:{line},{count}")?;
        }
        writeln!(out, "LF:{}", file.lines.len())?;
        writeln!(out, "LH:{}", file.lines.values().filter(|count| **count > 0).count())?;
        writeln!(out, "end_of_record")?;
    }
    Ok(())
}
