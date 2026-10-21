//! One small MASM program per code shape the reporter has to handle, each
//! checked against line, function, and branch counts computed by hand from
//! the coverable rule and the branch semantics in `docs/architecture.md`.
//!
//! The fixtures live in `tests/fixtures`; line numbers in the expectations are
//! 1-based lines of those files.

mod common;

use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

use common::{
    assemble_program, assert_functions, branches, default_report, file, fixture, functions,
    only_file, record_run, record_runs, report,
};
use miden_assembly::{
    Assembler, DefaultSourceManager, Linkage, ModuleParser, SourceManager, ast::ModuleKind,
};
use miden_core_lib::CoreLibrary;
use miden_cov::{FileCoverage, ReportFilter, SourceResolver};
use miden_cov_collector::{RunOutcome, dir::CoverageDir};
use miden_mast_package::{Package, QualifiedProcedureName};
use miden_processor::{DefaultHost, HostLibrary};

/// A coverage directory in a fresh temporary directory.
fn coverage_dir() -> (tempfile::TempDir, CoverageDir) {
    let tmp = tempfile::tempdir().unwrap();
    let coverage = CoverageDir::new(tmp.path().join("cov"));
    (tmp, coverage)
}

/// Parses `file` as the library module `path` with `source_manager`.
fn library_module(
    source_manager: &Arc<dyn SourceManager>,
    path: &str,
    file: &Path,
) -> Box<miden_assembly::ast::Module> {
    ModuleParser::new(Some(ModuleKind::Library))
        .parse_file(Some(miden_assembly::Path::new(path)), file, source_manager.clone())
        .unwrap()
}

/// Assembles the program at `program` with the library module at `library`,
/// named `library_path`, statically linked.
fn assemble_with_module(program: &Path, library_path: &str, library: &Path) -> Arc<Package> {
    let source_manager: Arc<dyn SourceManager> = Arc::new(DefaultSourceManager::default());
    let mut assembler = Assembler::new(source_manager.clone());
    assembler
        .compile_and_statically_link(library_module(&source_manager, library_path, library))
        .unwrap();
    Arc::from(assembler.assemble_program("program", program).unwrap())
}

/// Assembles the program at `source` with the core library linked
/// dynamically.
fn assemble_with_core(source: &Path) -> Arc<Package> {
    let assembler = Assembler::default()
        .with_package(CoreLibrary::default().package(), Linkage::Dynamic)
        .unwrap();
    Arc::from(assembler.assemble_program("program", source).unwrap())
}

/// A host with the core library and its debug info loaded.
fn core_host() -> DefaultHost {
    DefaultHost::default()
        .with_library(HostLibrary::from(CoreLibrary::default().package()))
        .unwrap()
}

/// The lines of `file` in `range`.
fn lines_in(file: &FileCoverage, range: std::ops::RangeInclusive<u32>) -> BTreeMap<u32, u64> {
    file.lines.range(range).map(|(line, count)| (*line, *count)).collect()
}

/// An `if.true` without `else`: the assembler emits an empty false arm, which
/// is a branch arm without a line of its own.
#[test]
fn if_without_else() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("if_without_else");
    record_runs(&coverage, &assemble_program(&source), &[&[1], &[0]]);
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    // Line 3 runs only when the condition holds; line 4 (`end`) and the empty
    // arm have no row.
    assert_eq!(file.lines, BTreeMap::from([(1, 2), (2, 2), (3, 1), (5, 2)]));
    assert_functions(&functions(file), &[("$main", 1, 2)]);
    assert_eq!(branches(file), [(2, true, [1, 1])]);
}

/// A loop inside an `if`, an `if` inside a loop body, and two sequential
/// `if`s, with runs that take different paths through each.
#[test]
fn nested_control_flow() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("nested");
    // Inputs are `[flag, n, m, a, b]`. Run A enters the `if` and loops twice,
    // loops three times over m = 2, 1, 0 (odd once, even twice), and takes
    // the first `if` and the second `else`. Run B skips the `if`, skips the
    // loop, and takes only the second `if`.
    record_runs(&coverage, &assemble_program(&source), &[&[1, 2, 3, 1, 0], &[0, 5, 0, 0, 1]]);
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    let expected_lines = BTreeMap::from([
        // loop_in_if: the loop guard (4) is evaluated twice true, once false.
        (1, 2),
        (2, 2),
        (3, 1),
        (4, 3),
        (5, 2),
        (6, 2),
        (9, 2),
        // if_in_loop: the guard (14) holds three times and fails twice.
        (12, 2),
        (13, 2),
        (14, 5),
        (15, 3),
        (16, 3),
        (17, 3),
        (18, 1),
        (20, 2),
        (22, 3),
        (24, 2),
        // two_ifs
        (27, 2),
        (28, 2),
        (29, 1),
        (31, 2),
        (32, 1),
        (34, 1),
        // begin
        (38, 2),
    ]);
    assert_eq!(file.lines, expected_lines);
    assert_functions(
        &functions(file),
        &[
            ("loop_in_if", 1, 2),
            ("if_in_loop", 12, 2),
            ("two_ifs", 27, 2),
            ("$main", 38, 2),
        ],
    );
    assert_eq!(
        branches(file),
        [
            (2, true, [1, 1]),
            (4, true, [2, 1]),
            (14, true, [3, 2]),
            (17, true, [1, 2]),
            (28, true, [1, 1]),
            (31, true, [1, 1]),
        ]
    );
}

/// `repeat.N` is unrolled by the assembler: every copy keeps the location of
/// the instruction it repeats.
#[test]
fn repeat_is_unrolled() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("repeat");
    // Inputs are the conditions of the two unrolled `if` copies.
    record_runs(&coverage, &assemble_program(&source), &[&[1, 0], &[1, 1]]);
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    // `add.1` (4) has three rows at one location, so it counts three per run.
    // The `if.true` (8) is evaluated twice per run, and line 9 counts the true
    // arms of both copies.
    assert_eq!(file.lines, BTreeMap::from([(1, 2), (2, 2), (4, 6), (6, 2), (8, 4), (9, 3)]));
    assert_functions(&functions(file), &[("$main", 1, 2)]);
    // The two copies of the `if` are one split node in the forest and one
    // source node in the debug info, reached from two joins, so they form a
    // single branch: true in three of four evaluations.
    assert_eq!(branches(file), [(8, true, [3, 1])]);
}

/// A procedure whose body starts with an `if.true` and one whose body starts
/// with a `while.true`: the function count stops at the leading split.
#[test]
fn body_starts_with_control_flow() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("body_starts_with_control");
    // Inputs are `[flag, cond, n]`. The loop runs three times in A, is
    // skipped in B, and runs once in C: four iterations, three exits.
    record_runs(&coverage, &assemble_program(&source), &[&[1, 1, 3], &[0, 0, 5], &[1, 1, 1]]);
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    let expected_lines = BTreeMap::from([
        (1, 3),
        (2, 3),
        (3, 2),
        (5, 1),
        (7, 3),
        (10, 3),
        // The loop guard counts evaluations: four true, three false.
        (11, 7),
        (12, 4),
        (13, 4),
        (15, 3),
        (18, 3),
    ]);
    assert_eq!(file.lines, expected_lines);
    // Each procedure ran once per run, however often its loop did.
    assert_functions(
        &functions(file),
        &[("starts_with_if", 1, 3), ("starts_with_while", 10, 3), ("$main", 18, 3)],
    );
    assert_eq!(branches(file), [(2, true, [2, 1]), (11, true, [4, 3])]);
}

/// Branch arms and a loop body that consist of a bare `exec`: each arm is
/// its own copy of the callee's tree, so the counts come out right from the
/// copies as well as from the edges the collector records.
#[test]
fn arms_that_are_procedure_calls() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("shared_arms");
    // Run 1: work(1), other(0), step(1); the first `if` calls work(1), the
    // second takes `else`; the loop calls step(0) and step(1). Run 2:
    // work(0), other(1), step(0); the first `if` calls other(1), the second
    // calls work(0); the loop is skipped.
    record_runs(
        &coverage,
        &assemble_program(&source),
        &[&[1, 0, 1, 1, 1, 0, 1, 0, 1, 1, 0], &[0, 1, 0, 0, 1, 1, 0, 0]],
    );
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    // work runs 4 times, other 3, step 4. The loop guard (39) holds twice
    // and fails twice.
    let expected_lines = BTreeMap::from([
        (1, 4),
        (2, 4),
        (3, 4),
        (4, 2),
        (6, 4),
        (9, 3),
        (10, 3),
        (11, 3),
        (12, 2),
        (14, 3),
        (17, 4),
        (18, 4),
        (19, 4),
        (20, 2),
        (22, 4),
        (25, 2),
        (29, 2),
        (34, 2),
        (37, 1),
        (39, 4),
    ]);
    assert_eq!(file.lines, expected_lines);
    assert_functions(
        &functions(file),
        &[("work", 1, 4), ("other", 9, 3), ("step", 17, 4), ("$main", 25, 2)],
    );
    // The copies of work, other, and step at the calls on lines 26 to 28 and
    // in both `if`s add up to the same counts.
    assert_eq!(
        branches(file),
        [
            (3, true, [2, 2]),
            (11, true, [2, 1]),
            (19, true, [2, 2]),
            (29, true, [1, 1]),
            (34, true, [1, 1]),
            (39, true, [2, 2]),
        ]
    );
}

/// A procedure made only of `exec` calls has no row of its own; its count
/// comes from the entries of its inlined copies.
#[test]
fn procedure_of_only_execs() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("exec_only");
    // Inputs are the conditions of leaf_a and leaf_b in the first and the
    // second call of only_execs.
    record_runs(&coverage, &assemble_program(&source), &[&[1, 0, 0, 1], &[0, 0, 0, 0]]);
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    let expected_lines =
        BTreeMap::from([(1, 4), (2, 4), (3, 1), (7, 4), (8, 4), (9, 1), (11, 3), (15, 4), (20, 2)]);
    assert_eq!(file.lines, expected_lines);
    assert_functions(
        &functions(file),
        &[("leaf_a", 1, 4), ("leaf_b", 7, 4), ("only_execs", 15, 4), ("$main", 20, 2)],
    );
    assert_eq!(branches(file), [(2, true, [1, 3]), (8, true, [1, 3])]);
}

/// A `call` inside an inlined procedure clones the callee's tree without an
/// inline call naming it, and a procedure both inlined and called: every
/// entry counts, whichever way the body was reached. `middle` has no row of
/// its own and a body of its own (two `exec`s), which is what lets the clone
/// be credited to it.
#[test]
fn procedures_reached_by_exec_and_call() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("exec_call");
    record_runs(&coverage, &assemble_program(&source), &[&[]]);
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    // `middle` runs twice through the one clone `outer`'s two `call`s share,
    // and twice through its own root, called from `main`; `leaf` runs twice
    // per `middle` and three times through `wrapped`.
    assert_eq!(
        file.lines,
        BTreeMap::from([(1, 11), (2, 11), (5, 4), (10, 1), (15, 3), (19, 1)])
    );
    assert_functions(
        &functions(file),
        &[
            ("leaf", 1, 11),
            ("middle", 5, 4),
            ("outer", 10, 1),
            ("wrapped", 15, 3),
            ("$main", 19, 1),
        ],
    );
}

/// The entry of a dynamically linked library whose body compiles to the same
/// MAST as another procedure of the library: the processor cannot pick the
/// entry's source root and tracks no source nodes for the invocation, so
/// everything below the entry reports zero. A VM gap recorded in 0018; the
/// expectations here are the correct ones, for when it closes.
#[test]
#[ignore = "VM gap (0018): ambiguous source root of a dynamically linked entry procedure"]
fn dynamic_library_entry_sharing_a_body() {
    let (_tmp, coverage) = coverage_dir();
    let library = fixture("ambiguous_entry_lib");
    let source = fixture("ambiguous_entry_program");
    let source_manager: Arc<dyn SourceManager> = Arc::new(DefaultSourceManager::default());
    let package: Arc<Package> = Arc::from(
        Assembler::new(source_manager.clone())
            .assemble_library(
                "ambiguous",
                library_module(&source_manager, "fixtures::ambiguous_entry_lib", &library),
                Vec::<Box<miden_assembly::ast::Module>>::new(),
            )
            .unwrap(),
    );
    let program = Arc::from(
        Assembler::new(source_manager)
            .with_package(package.clone(), Linkage::Dynamic)
            .unwrap()
            .assemble_program("program", source.clone())
            .unwrap(),
    );
    let host = DefaultHost::default().with_library(HostLibrary::from(package)).unwrap();
    record_run(&coverage, &program, host, "dynamic", &[0, 1]).unwrap();
    let report = default_report(&coverage);
    let file = file(&report, &library);
    // The flag is 0: the `else` arm runs.
    assert_eq!(file.lines, BTreeMap::from([(1, 1), (2, 1), (3, 0), (5, 1), (9, 1), (13, 1)]));
    assert_functions(&functions(file), &[("leaf", 1, 1), ("middle", 9, 1), ("entry", 13, 1)]);
}

/// A procedure re-exported from another module of a library: the assembler
/// records it twice, once per name, and both records must count as one.
/// `only_execs` has no row of its own and is reached by a `call` inside an
/// inlined copy, which is the case that needs the records folded.
#[test]
fn reexported_procedures_count_once() {
    let (_tmp, coverage) = coverage_dir();
    let inner = fixture("reexport_inner");
    let root = fixture("reexport_root");
    let source_manager: Arc<dyn SourceManager> = Arc::new(DefaultSourceManager::default());
    let package = Assembler::new(source_manager.clone())
        .assemble_library(
            "reexport",
            library_module(&source_manager, "fixtures::reexport_root", &root),
            vec![library_module(&source_manager, "fixtures::reexport_inner", &inner)],
        )
        .unwrap();
    let debug_info = package.debug_info().unwrap().unwrap();
    let records = debug_info
        .functions()
        .iter()
        .filter(|function| {
            debug_info
                .get_string(function.name_idx)
                .is_some_and(|name| name.ends_with("::only_execs"))
        })
        .count();
    assert_eq!(records, 2, "the re-export is recorded a second time");
    let entrypoint: QualifiedProcedureName = "fixtures::reexport_root::entry".parse().unwrap();
    let package = Arc::new(package.make_executable(&entrypoint).unwrap());
    record_runs(&coverage, &package, &[&[]]);
    let report = default_report(&coverage);
    let file = file(&report, &inner);
    assert_eq!(file.lines, BTreeMap::from([(1, 1), (2, 1), (5, 1), (6, 1), (9, 1), (14, 1)]));
    assert_functions(
        &functions(file),
        &[("leaf1", 1, 1), ("leaf2", 5, 1), ("only_execs", 9, 1), ("entry", 14, 1)],
    );
}

/// Two procedures whose bodies compile to the same MAST cannot be told apart
/// by their roots, so the one reached only by a `call` inside an inlined copy
/// is not credited. The gap 0019 records; this pins its shape so that a
/// lookup that picked one of the two by accident shows.
#[test]
fn a_shared_root_is_not_credited() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("shared_root");
    record_runs(&coverage, &assemble_program(&source), &[&[]]);
    let report = default_report(&coverage);
    let file = only_file(&report, &source);
    // `wrapper` comes first in the file, so a lookup that picked the first
    // record of a shared root would credit it, and one that picked the last
    // would credit `leaf` twice.
    assert_functions(
        &functions(file),
        &[("wrapper", 1, 0), ("leaf", 5, 1), ("outer", 9, 1), ("$main", 13, 1)],
    );
}

/// Two programs linking one library statically, in one coverage directory:
/// the library's records appear in both packages and agree, so a `call` in
/// each credits the callee, and the counts add up across the two runs. The
/// unused `other` is not linked in.
#[test]
fn two_programs_sharing_a_library() {
    let (_tmp, coverage) = coverage_dir();
    let library = fixture("lib2");
    for program in ["two_programs_a", "two_programs_b"] {
        let package = assemble_with_module(&fixture(program), "fixtures::lib2", &library);
        record_runs(&coverage, &package, &[&[]]);
    }
    let report = default_report(&coverage);
    let file = file(&report, &library);
    assert_functions(
        &functions(file),
        &[("leaf1", 1, 2), ("leaf2", 5, 2), ("only_execs", 9, 2), ("wrap", 14, 2)],
    );
}

/// A dynamically linked library both `exec`ed and `call`ed by the program: the
/// `exec` imports the library's whole function table into the program, so
/// the callee's root is recorded in both packages; the records agree, and
/// the `call` is credited to the library.
#[test]
fn a_dynamic_library_both_executed_and_called() {
    let (_tmp, coverage) = coverage_dir();
    let library = fixture("lib2");
    let source = fixture("exec_and_call_lib2");
    let source_manager: Arc<dyn SourceManager> = Arc::new(DefaultSourceManager::default());
    let package: Arc<Package> = Arc::from(
        Assembler::new(source_manager.clone())
            .assemble_library(
                "lib2",
                library_module(&source_manager, "fixtures::lib2", &library),
                Vec::<Box<miden_assembly::ast::Module>>::new(),
            )
            .unwrap(),
    );
    let program = Arc::from(
        Assembler::new(source_manager)
            .with_package(package.clone(), Linkage::Dynamic)
            .unwrap()
            .assemble_program("program", source)
            .unwrap(),
    );
    let host = DefaultHost::default().with_library(HostLibrary::from(package)).unwrap();
    record_run(&coverage, &program, host, "dynamic", &[]).unwrap();
    let report = default_report(&coverage);
    let file = file(&report, &library);
    assert_functions(
        &functions(file),
        &[
            ("leaf1", 1, 1),
            ("leaf2", 5, 1),
            ("only_execs", 9, 1),
            ("wrap", 14, 0),
            ("other", 18, 1),
        ],
    );
}

/// An inlined copy that begins in the middle of a block, after an `assert`:
/// the copy is entered once per execution of its first operation, not once
/// per start of the block, which the failing run would count.
#[test]
fn an_inlined_copy_after_a_failing_assert() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("assert_then_exec");
    let package = assemble_program(&source);
    record_run(&coverage, &package, DefaultHost::default(), "pass", &[1]).unwrap();
    record_run(&coverage, &package, DefaultHost::default(), "fail", &[0]).unwrap_err();
    let report = default_report(&coverage);
    let file = only_file(&report, &source);
    // The `exec` rows have no operations and are not coverable; `twice` is
    // credited on its definition line.
    assert_eq!(lines_in(file, 1..=6), BTreeMap::from([(1, 2), (2, 2), (5, 1)]));
    assert_functions(&functions(file), &[("leaf", 1, 2), ("twice", 5, 1), ("$main", 10, 2)]);
}
/// Procedures calling each other three levels deep, each with an `if`, so
/// every copy is a tree of its own: main calls a once, a calls b twice, b
/// calls c twice.
#[test]
fn calls_three_levels_deep() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("deep_calls");
    // Every procedure takes the input flag's value: all true, then all false.
    record_runs(&coverage, &assemble_program(&source), &[&[1], &[0]]);
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    let expected_lines = BTreeMap::from([
        (1, 8),
        (2, 8),
        (3, 4),
        (5, 4),
        (9, 4),
        (10, 4),
        (11, 4),
        (12, 2),
        (14, 4),
        (18, 2),
        (19, 2),
        (20, 2),
        (21, 1),
        (23, 2),
        (27, 2),
    ]);
    assert_eq!(file.lines, expected_lines);
    assert_functions(&functions(file), &[("c", 1, 8), ("b", 9, 4), ("a", 18, 2), ("$main", 27, 2)]);
    assert_eq!(branches(file), [(2, true, [4, 4]), (11, true, [2, 2]), (20, true, [1, 1])]);
}

/// Two procedures with identical bodies share one exec node; first is called
/// twice with a true condition, second once with a false one.
///
/// Each call gets a copy of the callee's source tree over the shared exec
/// nodes, so the counts stay apart. Attribution swapped between the two
/// procedures was reported earlier; it does not occur in this shape, with
/// the calls reordered, without the `push` before each call, or with `call`
/// instead of `exec`.
#[test]
fn deduplicated_bodies() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("dedup_bodies");
    record_runs(&coverage, &assemble_program(&source), &[&[]]);
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    let expected_lines = BTreeMap::from([
        (1, 2),
        (2, 2),
        (3, 2),
        (5, 0),
        (9, 1),
        (10, 1),
        (11, 0),
        (13, 1),
        (17, 1),
        (18, 1),
        (19, 1),
        (20, 1),
    ]);
    assert_eq!(file.lines, expected_lines);
    assert_functions(&functions(file), &[("first", 1, 2), ("second", 9, 1), ("$main", 17, 1)]);
    assert_eq!(branches(file), [(2, true, [2, 0]), (10, true, [0, 1])]);
}

/// A basic block longer than one operation batch, and a single line holding
/// many instructions.
#[test]
fn long_basic_block() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("long_block");
    record_runs(&coverage, &assemble_program(&source), &[&[]]);
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    // `begin`, `push.0`, 100 lines of `add.1`, `drop`, and one line with
    // `push.0`, 80 `add.1`, and `drop`: every line once.
    assert_eq!(file.lines, (1..=104).map(|line| (line, 1)).collect());
    assert_functions(&functions(file), &[("$main", 1, 1)]);
    assert!(file.branches.is_empty());
}

/// A program calling into a library module from another file: both files
/// are reported with their own counts.
#[test]
fn procedures_across_modules() {
    let (_tmp, coverage) = coverage_dir();
    let program = fixture("multi_module");
    let library = fixture("multi_module_lib");
    let package = assemble_with_module(&program, "fixtures::helpers", &library);
    // Inputs are `[g, f, x, y]`: run A calls twice(1, 0, 1) from local, run B
    // drops them. Both then call twice(1, 1, 0) from begin.
    record_runs(&coverage, &package, &[&[1, 1, 0, 1], &[0, 0, 0, 0]]);
    let report = default_report(&coverage);
    assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
    assert_eq!(report.files.len(), 2);

    let lib = file(&report, &library);
    assert_eq!(
        lib.lines,
        BTreeMap::from([(1, 3), (2, 3), (3, 3), (9, 6), (10, 6), (11, 6), (12, 3), (14, 6)])
    );
    assert_functions(&functions(lib), &[("twice", 1, 3), ("inner", 9, 6)]);
    assert_eq!(branches(lib), [(2, true, [3, 0]), (11, true, [3, 3])]);

    let main = file(&report, &program);
    assert_eq!(main.lines, BTreeMap::from([(3, 2), (4, 2), (7, 1), (11, 2), (13, 2)]));
    assert_functions(&functions(main), &[("local", 3, 2), ("$main", 11, 2)]);
    assert_eq!(branches(main), [(4, true, [1, 1])]);
}

/// The expected coverage of `library_entry.masm` after runs with `[2]` and
/// `[0]`, whether it ran as a library through an entrypoint or from a
/// program.
fn assert_library_entry(file: &FileCoverage) {
    let expected_lines = BTreeMap::from([
        (1, 2),
        (2, 2),
        (3, 2),
        (4, 1),
        (6, 1),
        (8, 2),
        (11, 1),
        (12, 1),
        (13, 3),
        (14, 2),
        (15, 2),
        (17, 1),
    ]);
    assert_eq!(file.lines, expected_lines);
    assert_functions(&functions(file), &[("entry", 1, 2), ("count", 11, 1)]);
    assert_eq!(branches(file), [(3, true, [1, 1]), (13, true, [2, 1])]);
}

/// A library package made executable through an entrypoint counts the same
/// as the program that calls the entrypoint.
#[test]
fn library_through_entrypoint() {
    let library = fixture("library_entry");
    let stacks: &[&[u64]] = &[&[2], &[0]];

    let (_tmp, coverage) = coverage_dir();
    let source_manager: Arc<dyn SourceManager> = Arc::new(DefaultSourceManager::default());
    let package = Assembler::new(source_manager.clone())
        .assemble_library(
            "entry_lib",
            library_module(&source_manager, "fixtures::entry_lib", &library),
            Vec::<Box<miden_assembly::ast::Module>>::new(),
        )
        .unwrap();
    let entrypoint: QualifiedProcedureName = "fixtures::entry_lib::entry".parse().unwrap();
    let package = Arc::new(package.make_executable(&entrypoint).unwrap());
    record_runs(&coverage, &package, stacks);
    assert_library_entry(only_file(&default_report(&coverage), &library));

    let (_tmp, coverage) = coverage_dir();
    let program = fixture("library_entry_program");
    let package = assemble_with_module(&program, "fixtures::entry_lib", &library);
    record_runs(&coverage, &package, stacks);
    let report = default_report(&coverage);
    assert_eq!(report.files.len(), 2);
    assert_library_entry(file(&report, &library));
    let main = file(&report, &program);
    assert_eq!(main.lines, BTreeMap::from([(3, 2)]));
    assert_functions(&functions(main), &[("$main", 3, 2)]);
}

/// The core library's `sys/mod.masm` among the files of a report.
fn core_sys(report: &miden_cov::Report) -> &FileCoverage {
    report
        .files
        .values()
        .find(|file| file.path.ends_with("asm/sys/mod.masm"))
        .unwrap_or_else(|| panic!("no core sys module in {:?}", report.files.keys()))
}

/// Keeps the program fixture `name` and the core library's `sys/mod.masm`.
fn program_and_core_sys(name: &str) -> ReportFilter {
    ReportFilter::new(&[format!("**/fixtures/{name}.masm"), "**/asm/sys/mod.masm".into()], &[])
        .unwrap()
}

/// A program calling a procedure of the dynamically linked core library:
/// the collector follows the switch to the library's forest, and the library
/// file is reported with its own counts.
#[test]
fn call_into_core_library() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("core_call");
    let package = assemble_with_core(&source);
    record_run(&coverage, &package, core_host(), "core", &[]).unwrap();
    let report = report(
        &coverage,
        &mut SourceResolver::new(None, vec![]),
        &program_and_core_sys("core_call"),
    );
    assert_eq!(report.files.len(), 2, "{:?}", report.files.keys());

    let main = file(&report, &source);
    assert_eq!(main.lines, BTreeMap::from([(3, 1), (4, 1)]));
    assert_functions(&functions(main), &[("$main", 3, 1)]);

    // Twelve elements above the sixteen: the first word is dropped before
    // the loop, which then drops the other two, one per iteration.
    // The debug info puts the definition on its first attribute,
    // `@locals(4)` on line 28, not on `pub proc` (29). Entered through the
    // program's external node, the procedure's root source node records no
    // start (its children do), so the count comes from the first row of the
    // body.
    let sys = core_sys(&report);
    let truncate = functions(sys).into_iter().find(|f| f.0.ends_with("truncate_stack")).unwrap();
    assert_eq!((truncate.1, truncate.2), (28, 1));
    assert_eq!(
        lines_in(sys, 28..=47),
        BTreeMap::from([(28, 1), (31, 1), (35, 1), (36, 3), (37, 2), (40, 2), (45, 1)])
    );
    assert!(branches(sys).contains(&(36, true, [2, 1])), "{:?}", branches(sys));
}

/// The nprime example from the VM repository, run for the first 50 primes
/// and for the first 2, merged.
#[test]
fn nprime() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("nprime");
    let package = assemble_with_core(&source);
    for (run, stack) in [[50], [2]].iter().enumerate() {
        record_run(&coverage, &package, core_host(), &run.to_string(), stack).unwrap();
    }
    let report = report(
        &coverage,
        &mut SourceResolver::new(None, vec![]),
        &program_and_core_sys("nprime"),
    );
    assert_eq!(report.files.len(), 2, "{:?}", report.files.keys());

    let main = file(&report, &source);
    let counts: Vec<(&str, u64)> =
        functions(main).into_iter().map(|(name, _, count)| (name, count)).collect();
    let expected = [
        ("append", 52),
        ("should_continue", 50),
        ("is_not_prime_should_continue", 429),
        ("is_not_prime", 113),
        ("next", 48),
        ("$main", 2),
    ];
    assert_eq!(counts.len(), expected.len(), "{counts:?}");
    for ((name, count), (suffix, expected)) in counts.iter().zip(expected) {
        assert!(name.ends_with(&format!("::{suffix}")), "{name} should end with {suffix}");
        assert_eq!(*count, expected, "{name}");
    }
    let arms: Vec<(u32, [u64; 2])> =
        branches(main).into_iter().map(|(line, _, arms)| (line, arms)).collect();
    assert_eq!(
        arms,
        [
            (67, [48, 381]),
            (75, [381, 48]),
            (84, [65, 316]),
            (107, [316, 113]),
            (133, [65, 48]),
            (172, [48, 2]),
        ]
    );

    let sys = core_sys(&report);
    let truncate = functions(sys).into_iter().find(|f| f.0.ends_with("truncate_stack")).unwrap();
    assert_eq!(truncate.2, 2);
    assert!(branches(sys).contains(&(36, true, [12, 2])), "{:?}", branches(sys));
}

/// A run that fails part way is still written; what ran before the failure
/// is counted and what comes after is not.
#[test]
fn failing_run() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("failing");
    // check takes the `if`, then asserts on 0.
    let result =
        record_run(&coverage, &assemble_program(&source), DefaultHost::default(), "fail", &[1, 0]);
    assert!(result.is_err());
    let runs = coverage.read_runs().unwrap();
    assert_eq!(runs.len(), 1);
    assert!(matches!(runs[0].outcome, RunOutcome::Failed(_)), "{:?}", runs[0].outcome);

    let report = default_report(&coverage);
    let file = only_file(&report, &source);
    // The failing `assert` (5) started, so it is hit; the rest of check (6)
    // and of begin (12) never ran.
    assert_eq!(
        file.lines,
        BTreeMap::from([(1, 1), (2, 1), (3, 1), (5, 1), (6, 0), (9, 1), (10, 1), (12, 0)])
    );
    assert_functions(&functions(file), &[("check", 1, 1), ("$main", 9, 1)]);
    assert_eq!(branches(file), [(2, true, [1, 0])]);
}

/// One source file stored under different paths by two packages is reported
/// once, with the counts of both.
#[test]
fn one_file_under_two_paths() {
    let (_tmp, coverage) = coverage_dir();
    let library = fixture("multi_module_lib");
    // The library linked into two programs, once through its absolute path
    // and once through a path relative to the crate, the test's current
    // directory.
    let absolute = assemble_with_module(&fixture("multi_module"), "fixtures::helpers", &library);
    let relative = assemble_with_module(
        &fixture("two_paths"),
        "fixtures::helpers",
        Path::new("tests/../tests/fixtures/multi_module_lib.masm"),
    );
    // multi_module drops its inputs and calls twice(1, 1, 0); two_paths calls
    // twice(1, 0, 0).
    record_runs(&coverage, &absolute, &[&[0, 0, 0, 0]]);
    record_runs(&coverage, &relative, &[&[]]);
    let report = default_report(&coverage);
    assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
    assert_eq!(report.files.len(), 3, "{:?}", report.files.keys());
    let copies: Vec<&FileCoverage> = report
        .files
        .values()
        .filter(|file| fs::canonicalize(&file.path).unwrap() == library)
        .collect();
    let [lib] = copies.as_slice() else {
        panic!("{} is reported {} times", library.display(), copies.len());
    };

    assert_eq!(
        lib.lines,
        BTreeMap::from([(1, 2), (2, 2), (3, 2), (9, 4), (10, 4), (11, 4), (12, 1), (14, 4)])
    );
    assert_functions(&functions(lib), &[("twice", 1, 2), ("inner", 9, 4)]);
    assert_eq!(branches(lib), [(2, true, [2, 0]), (11, true, [1, 3])]);
}

/// Arms that are bare procedure calls, where the procedures are also arms
/// of another `if` and are called directly. Each arm is a copy of its own
/// here, and the arm counts come out the same from the copies as from the
/// parent-to-child edges the collector records, which are what tells arms
/// apart when the callee is not copied. The loop body is a bare call to
/// `c`, which is also called directly.
#[test]
fn shared_arms_in_two_splits_and_a_loop() {
    let (_tmp, coverage) = coverage_dir();
    let source = fixture("shared_arms_twice");
    record_runs(&coverage, &assemble_program(&source), &[&[]]);
    let report = default_report(&coverage);
    let file = only_file(&report, &source);

    // pick: true -> a, false -> b (lines 35, 36). pick2: true -> b three
    // times (37-39). b once directly (40). c directly with 1 (41) and once
    // as the loop body with the 0 the first call left behind (44).
    assert_functions(
        &functions(file),
        &[
            ("a", 1, 1),
            ("b", 7, 5),
            ("c", 13, 2),
            ("pick", 20, 2),
            ("pick2", 27, 3),
            ("$main", 34, 1),
        ],
    );
    assert_eq!(
        branches(file),
        [
            (3, true, [1, 0]),
            (9, true, [0, 5]),
            (14, true, [1, 1]),
            (21, true, [1, 1]),
            (28, true, [3, 0]),
            (43, true, [1, 1]),
        ]
    );
    assert_eq!(
        file.lines,
        BTreeMap::from([
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 1),
            (7, 5),
            (8, 5),
            (9, 5),
            (10, 0),
            (13, 2),
            (14, 2),
            (15, 1),
            (17, 1),
            (20, 2),
            (21, 2),
            (27, 3),
            (28, 3),
            (34, 1),
            (35, 1),
            (36, 1),
            (37, 1),
            (38, 1),
            (39, 1),
            (41, 1),
            (42, 1),
            (43, 2),
            (46, 1),
        ])
    );
}
