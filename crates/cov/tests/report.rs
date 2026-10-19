//! Collects two runs of a MASM program assembled from a file and checks the
//! report, the LCOV output, and source resolution end to end.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use miden_assembly::Assembler;
use miden_core::Felt;
use miden_cov::{BranchCoverage, FunctionCoverage, Report, Snapshot, SourceResolver, build_report};
use miden_cov_collector::{Observer, RunOutcome, dir::CoverageDir, run_to_end};
use miden_mast_package::Package;
use miden_processor::{DefaultHost, FastProcessor, StackInputs};

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

/// Writes the fixture to `dir`, assembles it from the file, and records two
/// runs into a coverage directory: flag = 1, n = 3 doubles n and loops six
/// times; flag = 0, n = 0 takes the else arm and skips the loop.
fn collect(dir: &Path) -> (PathBuf, CoverageDir) {
    let source = dir.join("fixture.masm");
    fs::write(&source, SRC).unwrap();
    let package: Arc<Package> =
        Arc::from(Assembler::default().assemble_program("fixture", source.clone()).unwrap());
    let coverage = CoverageDir::new(dir.join("cov"));
    for (name, flag, n) in [("a", 1, 3), ("b", 0, 0)] {
        let mut host = DefaultHost::default();
        let mut processor = FastProcessor::new(
            StackInputs::new(&[Felt::new(flag).unwrap(), Felt::new(n).unwrap()]).unwrap(),
        );
        let ctx = processor.get_initial_resume_context_for_package(package.clone()).unwrap();
        let mut observer = Observer::new();
        run_to_end(&mut processor, &mut host, ctx, &mut observer).unwrap();
        coverage.write_output(&observer.finish(name.into(), RunOutcome::Ok)).unwrap();
    }
    (source, coverage)
}

/// Prints every located row of the snapshot, to diagnose unexpected lines.
fn print_rows(snapshot: &Snapshot) {
    for package in snapshot.packages.values() {
        let debug_info = &package.debug_info;
        for (node_id, node) in debug_info.nodes().iter().enumerate() {
            for row in &node.asm_ops {
                let location =
                    row.location_idx.into_option().and_then(|idx| debug_info.get_location(idx));
                println!(
                    "node {node_id} exec {:?} op {} cycles {} `{}` at {:?}",
                    node.exec_node,
                    row.op_idx,
                    row.num_cycles,
                    debug_info[row.op_name_idx],
                    location.map(|l| (l.uri.to_string(), l.start.0, l.end.0)),
                );
            }
        }
    }
}

fn report(coverage: &CoverageDir, resolver: &mut SourceResolver) -> Report {
    let snapshot = Snapshot::load(coverage).unwrap();
    assert_eq!(snapshot.runs.len(), 2);
    print_rows(&snapshot);
    build_report(&snapshot, resolver)
}

#[test]
fn report_follows_the_coverable_rule() {
    let tmp = tempfile::tempdir().unwrap();
    let (source, coverage) = collect(tmp.path());
    let report = report(&coverage, &mut SourceResolver::new(None, vec![]));
    println!("{report:#?}");

    assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
    assert_eq!(report.files.len(), 1);
    let file = report.files.values().next().unwrap();
    assert_eq!(file.path, source);

    // Line 16 (`exec.double`) has no row of its own. Line 1 (`proc double`)
    // has none either: the assembler emits a zero-operation `begin` row for a
    // procedure only when its body is a control node, and `double` is a single
    // basic block; the line is credited from the function's entries instead.
    // Lines 5 and 14 carry two `begin` rows each (one per join node), which is
    // why a line takes the maximum of its rows.
    let expected_lines = BTreeMap::from([
        (1, 1),
        (2, 1),
        (5, 2),
        (6, 2),
        (7, 2),
        (8, 6),
        (9, 6),
        (11, 2),
        (14, 2),
        (15, 2),
        (18, 1),
    ]);
    assert_eq!(file.lines, expected_lines);

    let functions: Vec<(&str, u32, u64)> = file
        .functions
        .iter()
        .map(|FunctionCoverage { name, line, count }| (name.as_str(), *line, *count))
        .collect();
    assert_eq!(functions.len(), 3, "{functions:?}");
    for ((name, line, count), (suffix, expected_line, expected_count)) in
        functions.iter().zip([("double", 1, 1), ("count_down", 5, 2), ("$main", 14, 2)])
    {
        assert!(name.ends_with(suffix), "{name} should end with {suffix}");
        assert_eq!((*line, *count), (expected_line, expected_count), "{name}");
    }

    let branches: Vec<(u32, bool, [u64; 2])> = file
        .branches
        .iter()
        .map(|BranchCoverage { line, executed, arms, .. }| (*line, *executed, *arms))
        .collect();
    assert_eq!(branches, [(7, true, [1, 1]), (15, true, [1, 1])]);

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

    let report = report(&coverage, &mut SourceResolver::new(Some(wrong_dir.path().into()), vec![]));
    assert!(report.files.is_empty());
    assert_eq!(report.unresolved.iter().collect::<Vec<_>>(), [&source.display().to_string()]);

    let mut summary = Vec::new();
    miden_cov::summary::write(&report, &mut summary).unwrap();
    let summary = String::from_utf8(summary).unwrap();
    assert!(summary.contains("Unresolved sources:"));
    assert!(summary.contains(&source.display().to_string()));
}
