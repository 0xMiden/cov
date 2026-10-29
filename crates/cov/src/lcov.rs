//! LCOV tracefile output, and input for comparing against a baseline.
//!
//! The format is the one `geninfo` writes and `genhtml` reads; see the
//! `geninfo(1)` manual page.

use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::PathBuf,
};

use crate::{BranchCoverage, Error, FileCoverage, FunctionCoverage, Report};

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

/// Reads a tracefile as [`write`] writes it back into a report keyed by the
/// `SF` paths.
///
/// Line (`DA`), function (`FN`, `FNDA`), and two-way branch (`BRDA`) records
/// are read; summary counts and records of other kinds are ignored. Branch
/// kinds are not in the format and read as unknown, and nothing is
/// unresolved.
pub fn read(text: &str) -> Result<Report, Error> {
    let mut report = Report::default();
    let mut file: Option<FileCoverage> = None;
    // Branch arms of the current file by line and block.
    let mut branches: BTreeMap<(u32, u32), BranchCoverage> = BTreeMap::new();
    for (idx, line) in text.lines().enumerate() {
        let invalid =
            |reason: &str| Error::InvalidLcov { line: idx + 1, reason: reason.to_owned() };
        let Some((tag, value)) = line.split_once(':') else {
            if line.trim() == "end_of_record" {
                let mut done =
                    file.take().ok_or_else(|| invalid("`end_of_record` without `SF`"))?;
                done.branches = std::mem::take(&mut branches).into_values().collect();
                done.functions.sort_by(|a, b| (a.line, &a.name).cmp(&(b.line, &b.name)));
                report.files.insert(done.path.display().to_string(), done);
            }
            continue;
        };
        if tag == "SF" {
            file = Some(FileCoverage {
                path: PathBuf::from(value),
                ..Default::default()
            });
            continue;
        }
        let Some(file) = file.as_mut() else {
            continue;
        };
        let number = |field: Option<&str>| -> Result<u64, Error> {
            field
                .and_then(|field| field.trim().parse().ok())
                .ok_or_else(|| invalid("bad number"))
        };
        match tag {
            "DA" => {
                let mut fields = value.split(',');
                let line = number(fields.next())? as u32;
                file.lines.insert(line, number(fields.next())?);
            },
            "FN" => {
                // `FN:<line>,<name>`, or `FN:<line>,<end line>,<name>` since
                // lcov 2.0.
                let (line, rest) = value.split_once(',').ok_or_else(|| invalid("bad `FN`"))?;
                let name = match rest.split_once(',') {
                    Some((end, name)) if end.parse::<u32>().is_ok() => name,
                    _ => rest,
                };
                file.functions.push(FunctionCoverage {
                    name: name.to_owned(),
                    line: number(Some(line))? as u32,
                    count: 0,
                });
            },
            "FNDA" => {
                let (count, name) = value.split_once(',').ok_or_else(|| invalid("bad `FNDA`"))?;
                let count = number(Some(count))?;
                let function = file
                    .functions
                    .iter_mut()
                    .find(|function| function.name == name)
                    .ok_or_else(|| invalid("`FNDA` without `FN`"))?;
                function.count = count;
            },
            "BRDA" => {
                let fields: Vec<&str> = value.split(',').collect();
                let [line, block, arm, taken] = fields[..] else {
                    return Err(invalid("bad `BRDA`"));
                };
                let (line, block) = (number(Some(line))? as u32, number(Some(block))? as u32);
                let arm = match arm {
                    "0" => 0,
                    "1" => 1,
                    _ => return Err(invalid("only two-way branches are supported")),
                };
                let branch = branches.entry((line, block)).or_insert(BranchCoverage {
                    line,
                    block,
                    executed: false,
                    arms: [0; 2],
                    kind: None,
                });
                if taken != "-" {
                    branch.executed = true;
                    branch.arms[arm] = number(Some(taken))?;
                }
            },
            _ => {},
        }
    }
    if file.is_some() {
        return Err(Error::InvalidLcov {
            line: text.lines().count(),
            reason: "the last record has no `end_of_record`".to_owned(),
        });
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BranchKind;

    #[test]
    fn read_inverts_write() {
        let file = FileCoverage {
            path: "/work/src/wallet.masm".into(),
            lines: BTreeMap::from([(1, 2), (2, 0), (7, 5)]),
            functions: vec![
                FunctionCoverage {
                    name: "::wallet::deposit".into(),
                    line: 1,
                    count: 2,
                },
                FunctionCoverage {
                    name: "::wallet::withdraw".into(),
                    line: 7,
                    count: 0,
                },
            ],
            branches: vec![
                BranchCoverage {
                    line: 2,
                    block: 14,
                    executed: true,
                    arms: [3, 0],
                    kind: None,
                },
                BranchCoverage {
                    line: 7,
                    block: 80,
                    executed: false,
                    arms: [0, 0],
                    kind: None,
                },
            ],
        };
        let mut report = Report::default();
        report.files.insert(file.path.display().to_string(), file);
        let mut text = Vec::new();
        write(&report, &mut text).unwrap();
        assert_eq!(read(&String::from_utf8(text).unwrap()).unwrap(), report);

        // The kind is not written.
        let mut known = report.clone();
        known.files.values_mut().next().unwrap().branches[0].kind = Some(BranchKind::If);
        let mut text = Vec::new();
        write(&known, &mut text).unwrap();
        assert_eq!(read(&String::from_utf8(text).unwrap()).unwrap(), report);
    }

    #[test]
    fn read_accepts_other_writers() {
        let report = read(
            "VER:2\nTN:x\nSF:a.masm\nFN:3,9,f\nFNL:0,3,9\nFNDA:4,f\nDA:3,4,abc\nLF:1\n\
             end_of_record\n",
        )
        .unwrap();
        let file = &report.files["a.masm"];
        assert_eq!(file.functions, [FunctionCoverage { name: "f".into(), line: 3, count: 4 }]);
        assert_eq!(file.lines, BTreeMap::from([(3, 4)]));
    }

    #[test]
    fn read_rejects_malformed_records() {
        for (text, line) in [
            ("SF:a\nDA:x,1\nend_of_record\n", 2),
            ("SF:a\nBRDA:1,0,2,1\nend_of_record\n", 2),
            ("SF:a\nFNDA:1,f\nend_of_record\n", 2),
            ("end_of_record\n", 1),
            ("SF:a\nDA:1,1\n", 2),
        ] {
            let error = read(text).unwrap_err();
            assert!(error.to_string().contains(&format!("line {line}")), "{text:?}: {error}");
        }
    }
}
