//! Collects runs of MASM programs assembled from files and checks the report,
//! the LCOV output, file filters, and source resolution end to end.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use miden_assembly::Assembler;
use miden_core::{Felt, mast::MastNode};
use miden_cov::{
    BranchCoverage, FileCoverage, FunctionCoverage, Report, ReportFilter, Snapshot, SourceResolver,
    build_report,
};
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

/// Writes `src` to `dir/name.masm`, assembles it from the file, and records
/// one run per stack into `dir/cov`.
fn collect_runs(dir: &Path, name: &str, src: &str, stacks: &[&[u64]]) -> (PathBuf, CoverageDir) {
    let source = dir.join(format!("{name}.masm"));
    fs::write(&source, src).unwrap();
    let package: Arc<Package> =
        Arc::from(Assembler::default().assemble_program(name, source.clone()).unwrap());
    let coverage = CoverageDir::new(dir.join("cov"));
    for (run, stack) in stacks.iter().enumerate() {
        let stack: Vec<Felt> = stack.iter().map(|value| Felt::new(*value).unwrap()).collect();
        let mut host = DefaultHost::default();
        let mut processor = FastProcessor::new(StackInputs::new(&stack).unwrap());
        let ctx = processor.get_initial_resume_context_for_package(package.clone()).unwrap();
        let mut observer = Observer::new();
        run_to_end(&mut processor, &mut host, ctx, &mut observer).unwrap();
        coverage
            .write_output(&observer.finish(run.to_string(), RunOutcome::Ok))
            .unwrap();
    }
    (source, coverage)
}

/// Records two runs of [`SRC`]: flag = 1, n = 3 doubles n and loops six
/// times; flag = 0, n = 0 takes the else arm and skips the loop.
fn collect(dir: &Path) -> (PathBuf, CoverageDir) {
    collect_runs(dir, "fixture", SRC, &[&[1, 3], &[0, 0]])
}

/// Prints every source node with its kind and starts, and every row with its
/// hits and location, to explain unexpected counts.
fn print_rows(snapshot: &Snapshot) {
    for (commitment, package) in &snapshot.packages {
        let debug_info = &package.debug_info;
        let hits = &snapshot.merged[commitment];
        for (node_id, node) in debug_info.nodes().iter().enumerate() {
            let kind = match package.forest.get_node_by_id(node.exec_node) {
                Some(MastNode::Block(_)) => "block",
                Some(MastNode::Join(_)) => "join",
                Some(MastNode::Split(_)) => "split",
                Some(MastNode::Loop(_)) => "loop",
                Some(MastNode::Call(_)) => "call",
                Some(MastNode::Dyn(_)) => "dyn",
                Some(MastNode::External(_)) => "external",
                None => "missing",
            };
            let starts = hits.node_starts.get(&(node_id as u32)).copied().unwrap_or(0);
            println!("node {node_id} {kind} starts {starts} children {:?}", node.children);
            for row in &node.asm_ops {
                let location =
                    row.location_idx.into_option().and_then(|idx| debug_info.get_location(idx));
                let op_hits = hits.op_hits.get(&(node_id as u32, row.op_idx)).copied().unwrap_or(0);
                println!(
                    "  op {} cycles {} hits {op_hits} `{}` in {} at {:?}",
                    row.op_idx,
                    row.num_cycles,
                    debug_info[row.op_name_idx],
                    debug_info[row.context_name_idx],
                    location.map(|l| (l.start.0, l.end.0)),
                );
            }
        }
        for function in debug_info.functions() {
            println!(
                "function {} line {} root {:?}",
                debug_info[function.name_idx],
                function.line.0 + 1,
                function.source_node.into_option(),
            );
        }
    }
}

fn report(coverage: &CoverageDir, resolver: &mut SourceResolver, filter: &ReportFilter) -> Report {
    let snapshot = Snapshot::load(coverage).unwrap();
    print_rows(&snapshot);
    build_report(&snapshot, resolver, filter)
}

/// The one file of `report`, which must be `source`.
fn only_file<'a>(report: &'a Report, source: &Path) -> &'a FileCoverage {
    assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
    assert_eq!(report.files.len(), 1);
    let file = report.files.values().next().unwrap();
    assert_eq!(file.path, source);
    file
}

/// The functions of `file` as `(name, line, count)`.
fn functions(file: &FileCoverage) -> Vec<(&str, u32, u64)> {
    file.functions
        .iter()
        .map(|FunctionCoverage { name, line, count }| (name.as_str(), *line, *count))
        .collect()
}

/// Checks `functions` against `(name suffix, line, count)` in order.
fn assert_functions(functions: &[(&str, u32, u64)], expected: &[(&str, u32, u64)]) {
    assert_eq!(functions.len(), expected.len(), "{functions:?}");
    for ((name, line, count), (suffix, expected_line, expected_count)) in
        functions.iter().zip(expected)
    {
        assert!(name.ends_with(suffix), "{name} should end with {suffix}");
        assert_eq!((*line, *count), (*expected_line, *expected_count), "{name}");
    }
}

/// The branches of `file` as `(line, executed, arms)`.
fn branches(file: &FileCoverage) -> Vec<(u32, bool, [u64; 2])> {
    file.branches
        .iter()
        .map(|BranchCoverage { line, executed, arms, .. }| (*line, *executed, *arms))
        .collect()
}

#[test]
fn report_follows_the_coverable_rule() {
    let tmp = tempfile::tempdir().unwrap();
    let (source, coverage) = collect(tmp.path());
    let report =
        report(&coverage, &mut SourceResolver::new(None, vec![]), &ReportFilter::default());
    println!("{report:#?}");
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
    let report =
        report(&coverage, &mut SourceResolver::new(None, vec![]), &ReportFilter::default());
    println!("{report:#?}");
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
