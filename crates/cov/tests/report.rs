//! Collects runs of MASM programs assembled from files and checks the report,
//! the LCOV output, file filters, and source resolution end to end.

mod common;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use common::{
    assert_functions, branches, collect_runs, default_report, functions, only_file, report,
};
use miden_cov::{ReportFilter, SourceResolver};
use miden_cov_collector::dir::CoverageDir;

/// Two procedures, an if/else, and a loop guarded by the split the assembler
/// wraps around every `while.true`. `proc double` is line 1.
const SRC: &str = "\
proc double
    mul.2
end

proc count_down
    dup neq.0
    while.true
        sub.1
        dup neq.0
    end
    drop
end

begin
    if.true
        exec.double
    else
        push.7 drop
    end
    exec.count_down
end
";

/// Single-block procedures the assembler inlines at their call sites, one of
/// them called from a loop body. `proc tiny` is line 1. `begin` increments
/// the loop counter once; `body` leaves it alone.
const INLINED_SRC: &str = "\
proc tiny
    add.1
end

proc body
    push.1 drop
    push.2 drop
    push.3
    exec.tiny
    exec.tiny drop
end

proc looper
    dup neq.0
    while.true
        sub.1
        exec.body
        dup neq.0
    end
    drop
end

begin
    exec.tiny
    exec.looper
end
";

/// Records two runs of [`SRC`]: flag = 1, n = 3 doubles n and loops six
/// times; flag = 0, n = 0 takes the else arm and skips the loop.
fn collect(dir: &Path) -> (PathBuf, CoverageDir) {
    collect_runs(dir, "fixture", SRC, &[&[1, 3], &[0, 0]])
}

#[test]
fn report_follows_the_coverable_rule() {
    let tmp = tempfile::tempdir().unwrap();
    let (source, coverage) = collect(tmp.path());
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    // Line 16 (`exec.double`) has no row of its own. Lines 1, 5, and 14 (the
    // procedure definitions) are credited from the function counts: `double`
    // is a single basic block with no row there, and the `begin` rows of the
    // other two sit on join nodes, which are not coverable. Line 7 counts
    // condition evaluations: run A loops six times and exits once, run B
    // skips the loop.
    let expected_lines = BTreeMap::from([
        (1, 1),
        (2, 1),
        (5, 2),
        (6, 2),
        (7, 8),
        (8, 6),
        (9, 6),
        (11, 2),
        (14, 2),
        (15, 2),
        (18, 1),
    ]);
    assert_eq!(file.lines, expected_lines);

    let functions = functions(file);
    assert_functions(&functions, &[("double", 1, 1), ("count_down", 5, 2), ("$main", 14, 2)]);

    // The loop guard's arms are the condition's outcomes: true six times,
    // false once on run A's exit and once on run B's skip.
    assert_eq!(branches(file), [(7, true, [6, 2]), (15, true, [1, 1])]);

    let mut lcov = Vec::new();
    miden_cov::lcov::write(&report, &mut lcov).unwrap();
    let lcov = String::from_utf8(lcov).unwrap();
    println!("{lcov}");
    let lcov_lines: Vec<&str> = lcov.lines().collect();
    assert!(lcov_lines.contains(&"DA:8,6"));
    assert!(lcov_lines.iter().any(|line| line.starts_with("BRDA:15,")));
    let double = functions[0].0;
    assert!(lcov_lines.contains(&format!("FNDA:1,{double}").as_str()));
    assert_eq!(lcov_lines.last(), Some(&"end_of_record"));
    assert_eq!(
        lcov_lines.iter().filter(|line| line.starts_with("SF:")).count(),
        lcov_lines.iter().filter(|line| **line == "end_of_record").count()
    );
    genhtml(tmp.path(), &lcov);

    let mut summary = Vec::new();
    miden_cov::summary::write(&report, &mut summary).unwrap();
    let summary = String::from_utf8(summary).unwrap();
    println!("{summary}");
    assert!(summary.contains("TOTAL"));
    assert!(!summary.contains("Unresolved sources:"));
}

#[test]
fn inlined_procedures_are_counted_at_every_copy() {
    let tmp = tempfile::tempdir().unwrap();
    // `begin` increments 2 to 3, so the loop runs three times.
    let (source, coverage) = collect_runs(tmp.path(), "inlined", INLINED_SRC, &[&[2]]);
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    // `tiny` and `body` are inlined into the loop body block, and `tiny`
    // also into `begin`'s first block. `add.1` (line 2) runs once from
    // `begin` and twice per iteration. The loop condition (line 15) is
    // evaluated three times true and once false. Definition lines carry the
    // function counts, not the loop body's; `begin` (line 23) only has rows
    // of the inlined `tiny` and on joins, so its count is its root's starts.
    let expected_lines = BTreeMap::from([
        (1, 7),
        (2, 7),
        (5, 3),
        (6, 3),
        (7, 3),
        (8, 3),
        (10, 3),
        (13, 1),
        (14, 1),
        (15, 4),
        (16, 3),
        (18, 3),
        (20, 1),
        (23, 1),
    ]);
    assert_eq!(file.lines, expected_lines);
    assert_functions(
        &functions(file),
        &[("tiny", 1, 7), ("body", 5, 3), ("looper", 13, 1), ("$main", 23, 1)],
    );
    assert_eq!(branches(file), [(15, true, [3, 1])]);

    let mut lcov = Vec::new();
    miden_cov::lcov::write(&report, &mut lcov).unwrap();
    genhtml(tmp.path(), &String::from_utf8(lcov).unwrap());
}

#[test]
fn filters_select_reported_files() {
    let tmp = tempfile::tempdir().unwrap();
    let (source, coverage) = collect(tmp.path());
    let base = tmp.path();
    let absolute = format!("{}/*.masm", base.display());
    let filtered = |include: &[&str], exclude: &[&str]| {
        let filter = ReportFilter::new(include, exclude).unwrap().relative_to(base);
        report(&coverage, &mut SourceResolver::new(None, vec![]), &filter)
    };

    // Relative globs match the path relative to the base; absolute ones the
    // resolved path.
    for include in ["fixture.masm", "*.masm", absolute.as_str()] {
        only_file(&filtered(&[include], &[]), &source);
    }
    assert!(filtered(&["src/**"], &[]).files.is_empty());
    assert!(filtered(&[], &["fixture.masm"]).files.is_empty());
    assert!(filtered(&[], &[absolute.as_str()]).files.is_empty());
    only_file(&filtered(&[], &["other/**"]), &source);
    assert!(filtered(&["*.masm"], &["fixture.*"]).files.is_empty());
    only_file(&filtered(&["*.masm", "src/**"], &["other.masm"]), &source);
}

#[test]
fn filtered_out_sources_are_not_unresolved() {
    let tmp = tempfile::tempdir().unwrap();
    let (source, coverage) = collect(tmp.path());
    fs::rename(&source, tmp.path().join("moved.masm")).unwrap();

    let report_with =
        |filter: &ReportFilter| report(&coverage, &mut SourceResolver::new(None, vec![]), filter);
    let unfiltered = report_with(&ReportFilter::default());
    assert_eq!(
        unfiltered.unresolved.iter().collect::<Vec<_>>(),
        [&source.display().to_string()]
    );
    let excluded = ReportFilter::new(&[], &["**/fixture.masm"]).unwrap();
    assert!(report_with(&excluded).unresolved.is_empty());
}

/// Renders `lcov` with genhtml and checks that it accepts the tracefile
/// without warnings.
fn genhtml(dir: &Path, lcov: &str) {
    let tracefile = dir.join("lcov.info");
    fs::write(&tracefile, lcov).unwrap();
    let output = match Command::new("genhtml")
        .arg("--branch-coverage")
        .arg("--output-directory")
        .arg(dir.join("html"))
        .arg(&tracefile)
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            println!("note: skipping the genhtml check, genhtml is not runnable: {e}");
            return;
        },
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "genhtml failed: {stderr}");
    assert!(!stderr.contains("WARNING"), "genhtml warned: {stderr}");
}

#[test]
fn unresolved_sources_are_listed_not_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let (source, coverage) = collect(tmp.path());
    fs::rename(&source, tmp.path().join("moved.masm")).unwrap();
    let wrong_dir = tempfile::tempdir().unwrap();

    let report = report(
        &coverage,
        &mut SourceResolver::new(Some(wrong_dir.path().into()), vec![]),
        &ReportFilter::default(),
    );
    assert!(report.files.is_empty());
    assert_eq!(report.unresolved.iter().collect::<Vec<_>>(), [&source.display().to_string()]);

    let mut summary = Vec::new();
    miden_cov::summary::write(&report, &mut summary).unwrap();
    let summary = String::from_utf8(summary).unwrap();
    assert!(summary.contains("Unresolved sources:"));
    assert!(summary.contains(&source.display().to_string()));
}
