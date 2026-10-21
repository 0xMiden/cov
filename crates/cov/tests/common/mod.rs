//! Helpers shared by the reporter's integration tests: assembling MASM,
//! running it under the collector, and reading the report back.

// Each test binary uses its own subset of the helpers.
#![allow(dead_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use miden_assembly::Assembler;
use miden_core::{Felt, mast::MastNode};
use miden_cov::{
    BranchCoverage, FileCoverage, FunctionCoverage, Report, ReportFilter, Snapshot, SourceResolver,
    build_report,
};
use miden_cov_collector::{Collector, RunOutcome, Stepper, dir::CoverageDir};
use miden_mast_package::Package;
use miden_processor::{DefaultHost, ExecutionError, FastProcessor, StackInputs};

/// The path of the fixture `tests/fixtures/<name>.masm`.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.masm"))
}

/// Assembles the program at `source` with the default assembler.
pub fn assemble_program(source: &Path) -> Arc<Package> {
    let name = source.file_stem().unwrap().to_string_lossy().into_owned();
    Arc::from(Assembler::default().assemble_program(name, source).unwrap())
}

/// Runs `package` on `stack` (first element on top) with `host` under the
/// collector and writes the run, named `name`, into `coverage` whatever the
/// outcome.
pub fn record_run(
    coverage: &CoverageDir,
    package: &Arc<Package>,
    mut host: DefaultHost,
    name: &str,
    stack: &[u64],
) -> Result<(), ExecutionError> {
    let stack: Vec<Felt> = stack.iter().map(|value| Felt::new(*value).unwrap()).collect();
    let processor = FastProcessor::new(StackInputs::new(&stack).unwrap());
    let mut stepper = Stepper::for_package(processor, package.clone()).unwrap();
    let mut collector = Collector::new();
    let result = stepper.run_sync(&mut host, |cycle| collector.on_cycle(cycle));
    coverage
        .write_output(&collector.finish(name.to_owned(), RunOutcome::of(&result)))
        .unwrap();
    result
}

/// Runs `package` once per stack with a default host, each run succeeding.
pub fn record_runs(coverage: &CoverageDir, package: &Arc<Package>, stacks: &[&[u64]]) {
    for (run, stack) in stacks.iter().enumerate() {
        record_run(coverage, package, DefaultHost::default(), &run.to_string(), stack).unwrap();
    }
}

/// Writes `src` to `dir/name.masm`, assembles it from the file, and records
/// one run per stack into `dir/cov`.
pub fn collect_runs(
    dir: &Path,
    name: &str,
    src: &str,
    stacks: &[&[u64]],
) -> (PathBuf, CoverageDir) {
    let source = dir.join(format!("{name}.masm"));
    fs::write(&source, src).unwrap();
    let coverage = CoverageDir::new(dir.join("cov"));
    record_runs(&coverage, &assemble_program(&source), stacks);
    (source, coverage)
}

/// Prints every source node with its kind and starts, and every row with its
/// hits, name, procedure, and location, to explain unexpected counts.
///
/// Nodes whose rows all lie in files that `filter` drops are skipped, which
/// keeps the output of a run through the core library readable.
pub fn print_rows(snapshot: &Snapshot, filter: &ReportFilter) {
    for (commitment, package) in &snapshot.packages {
        let debug_info = &package.debug_info;
        let Some(hits) = snapshot.merged.get(commitment) else {
            continue;
        };
        println!("package {commitment:?}");
        for (node_id, node) in debug_info.nodes().iter().enumerate() {
            let mut files = node
                .asm_ops
                .iter()
                .filter_map(|row| row.location_idx.into_option())
                .filter_map(|idx| debug_info.get_location(idx))
                .peekable();
            if files.peek().is_some()
                && !files.any(|location| filter.keeps(Path::new(location.uri.as_str())))
            {
                continue;
            }
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
            println!(
                "node {node_id} {kind} exec {:?} starts {starts} children {:?}",
                node.exec_node, node.children
            );
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
                    location.map(|l| (l.uri.as_str().to_owned(), l.start.0, l.end.0)),
                );
            }
        }
        for function in debug_info.functions() {
            let path = debug_info
                .get_file(function.file_idx)
                .and_then(|file| debug_info.get_string(file.path_idx));
            if path.is_some_and(|path| !filter.keeps(Path::new(path.as_ref()))) {
                continue;
            }
            println!(
                "function {} line {} root {:?}",
                debug_info[function.name_idx],
                function.line.0 + 1,
                function.source_node.into_option(),
            );
        }
    }
}

/// Loads `coverage`, prints its rows, and builds the report.
pub fn report(
    coverage: &CoverageDir,
    resolver: &mut SourceResolver,
    filter: &ReportFilter,
) -> Report {
    let snapshot = Snapshot::load(coverage).unwrap();
    print_rows(&snapshot, filter);
    let report = build_report(&snapshot, resolver, filter);
    println!("{report:#?}");
    report
}

/// The report of `coverage` with sources resolved from the current directory
/// and no filter.
pub fn default_report(coverage: &CoverageDir) -> Report {
    report(coverage, &mut SourceResolver::new(None, vec![]), &ReportFilter::default())
}

/// The one file of `report`, which must be `source`.
pub fn only_file<'a>(report: &'a Report, source: &Path) -> &'a FileCoverage {
    assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
    assert_eq!(report.files.len(), 1, "{:?}", report.files.keys());
    let file = report.files.values().next().unwrap();
    assert_eq!(file.path, source);
    file
}

/// The file of `report` found at `source`.
pub fn file<'a>(report: &'a Report, source: &Path) -> &'a FileCoverage {
    report
        .files
        .values()
        .find(|file| file.path == source)
        .unwrap_or_else(|| panic!("{} is not in {:?}", source.display(), report.files.keys()))
}

/// The functions of `file` as `(name, line, count)`.
pub fn functions(file: &FileCoverage) -> Vec<(&str, u32, u64)> {
    file.functions
        .iter()
        .map(|FunctionCoverage { name, line, count }| (name.as_str(), *line, *count))
        .collect()
}

/// Checks `functions` against `(name suffix, line, count)` in order.
pub fn assert_functions(functions: &[(&str, u32, u64)], expected: &[(&str, u32, u64)]) {
    assert_eq!(functions.len(), expected.len(), "{functions:?}");
    for ((name, line, count), (suffix, expected_line, expected_count)) in
        functions.iter().zip(expected)
    {
        assert!(name.ends_with(suffix), "{name} should end with {suffix}");
        assert_eq!((*line, *count), (*expected_line, *expected_count), "{name}");
    }
}

/// The branches of `file` as `(line, executed, arms)`.
pub fn branches(file: &FileCoverage) -> Vec<(u32, bool, [u64; 2])> {
    file.branches
        .iter()
        .map(|BranchCoverage { line, executed, arms, .. }| (*line, *executed, *arms))
        .collect()
}
