//! Differential test of source attribution: the collector's counts against
//! the debugger engine's row attribution.
//!
//! The collector has no reading of the continuation stream of its own: it
//! counts the per-cycle records the engine hands out (source node, padded
//! operation index, node starts). The engine's `DebugExecutor` separately
//! exposes, after each `step()`, the operation just executed (`current_op`)
//! and the row it attributes it to (`current_asmop`), computed from the padded
//! operation index through the asm-op lookup. Every program here runs through
//! the collector driven by the engine's `Stepper` and through the
//! `DebugExecutor` with the same package and inputs, and the attributions must
//! agree. This is the contract test of the engine's per-cycle hook and of
//! `Stepper` against `DebugExecutor`, and a disagreement shows up without
//! hand-written expectations.
//!
//! # The engine side
//!
//! After every step the cycle is skipped when it executed no operation
//! (`current_op` is `None`: a control-flow cycle), when it executed a `Noop`
//! (batch padding), or when it has no row or the row no location. Otherwise
//! the cycle counts against the row's location and its `num_cycles`.
//!
//! The engine finds a cycle's row as the last row of the source node at or
//! before the operation's padded index. An operation that belongs to no row,
//! such as the frame setup of a procedure with locals inlined into its
//! caller's block, is therefore charged to the preceding row.
//!
//! # The checks
//!
//! 1. **Cycles.** Our op hits, charged to rows by the engine's rule (every non-`Noop` operation of
//!    a block to the last row at or before it), must equal the engine's cycles per (location,
//!    `num_cycles`) exactly. This checks every operation the collector recorded, not only the first
//!    of each row, and needs no division.
//! 2. **Executions.** A location's execution count by the engine is its cycles divided by
//!    `num_cycles`; ours is the reporter's row count before line aggregation: the hits of the first
//!    operation of every row with operations at that location, summed over rows (inlined copies)
//!    and packages. The two must be equal.
//!
//! # Exclusions
//!
//! - Rows with zero operations (`begin`, `exec`, `if.true`, `while.true`, ...) are left out of
//!   check 2 on our side: no operation belongs to them, so the engine never reports them. Locations
//!   never executed are dropped on our side: the engine only sees what ran.
//! - Check 2 skips a location when an operation charged to it by the engine belongs to no row (see
//!   above), when its cycles are not a multiple of `num_cycles`, or when the engine charges cycles
//!   to a zero-operation row there. Its cycles do not measure executions; check 1 still covers it.
//!   Skipped locations are printed.
//! - The engine's alternative of counting a location whenever the reported (location, procedure,
//!   instruction) changes is not used: it undercounts identical consecutive rows (`repeat.4 dupw.3`
//!   is one change for four executions) and double counts rows whose operations straddle a batch
//!   boundary, where a `RESPAN` control-flow cycle interrupts them.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Instant,
};

use miden_assembly::{Assembler, Linkage};
use miden_core_lib::CoreLibrary;
use miden_cov::Snapshot;
use miden_cov_collector::{Collector, RunOutcome, dir::CoverageDir};
use miden_debug_engine::{
    ExecutionConfig, Executor, Stepper,
    debug_types::{DefaultSourceManager, Location},
    processor::{
        DefaultHost, ExecutionOptions, FastProcessor, Felt, HostLibrary, StackInputs,
        advice::{AdviceInputs, AdviceStack},
        mast::MastNode,
        operation::Operation,
    },
};
use miden_mast_package::{
    Package,
    debug_info::{DebugSourceAsmOp, PackageDebugInfo},
};

/// A source location: file, start byte, end byte.
type LocationKey = (String, u32, u32);

/// The engine's unit of attribution: a row's location and operation count.
type RowKey = (LocationKey, u8);

/// A row as written, for messages.
#[derive(Debug)]
struct Row {
    /// The instruction.
    op: String,
    /// The procedure the row belongs to.
    context: String,
}

/// How one run's cycles were attributed to rows.
#[derive(Debug, Default)]
struct Attribution {
    /// Operation cycles per row kind, by the engine's rule.
    cycles: BTreeMap<RowKey, u64>,
    /// The row behind each location.
    rows: BTreeMap<LocationKey, Row>,
}

/// Our attribution of one run, read from a coverage directory.
#[derive(Debug, Default)]
struct Collected {
    /// The engine's rule applied to our op hits.
    attribution: Attribution,
    /// Executions per location by the reporter's row rule.
    executions: BTreeMap<LocationKey, u64>,
    /// Cycles the engine's rule charges to a row although the operation
    /// belongs to no row, per row kind.
    rowless: BTreeMap<RowKey, u64>,
}

/// A program to run, with its inputs.
struct Program {
    name: &'static str,
    /// Path relative to `tests/fixtures`.
    source: &'static str,
    /// Operand stack, top first.
    stack: Vec<u64>,
    /// Advice stack, top first.
    advice: Vec<u64>,
}

impl Program {
    fn new(name: &'static str, source: &'static str, stack: Vec<u64>) -> Self {
        Self { name, source, stack, advice: Vec::new() }
    }

    fn with_advice(mut self, advice: Vec<u64>) -> Self {
        self.advice = advice;
        self
    }

    fn path(&self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(self.source)
    }

    /// Assembles the program from its path, so locations carry the real file
    /// path, dynamically linked against the core library.
    fn assemble(&self) -> Arc<Package> {
        let package = Assembler::default()
            .with_package(CoreLibrary::default().package(), Linkage::Dynamic)
            .unwrap()
            .assemble_program(self.name, self.path())
            .unwrap();
        Arc::from(package)
    }

    fn stack_inputs(&self) -> StackInputs {
        StackInputs::new(&felts(&self.stack)).unwrap()
    }

    fn advice_inputs(&self) -> AdviceInputs {
        AdviceInputs::default().with_stack(AdviceStack::from(felts(&self.advice)))
    }
}

fn felts(values: &[u64]) -> Vec<Felt> {
    values.iter().map(|value| Felt::new(*value).unwrap()).collect()
}

fn location_key(location: &Location) -> LocationKey {
    (location.uri.as_str().to_owned(), location.start.0, location.end.0)
}

/// Runs `program` in the debugger engine and records which row it charged
/// every operation cycle to.
fn engine_attribution(program: &Program, package: &Arc<Package>) -> Attribution {
    let mut executor = Executor::from_config(ExecutionConfig {
        inputs: program.stack_inputs(),
        advice_inputs: program.advice_inputs(),
        options: ExecutionOptions::default(),
    });
    executor.with_package(CoreLibrary::default().package()).unwrap();
    let mut exec = executor.into_debug(package.clone(), Arc::new(DefaultSourceManager::default()));

    let mut attribution = Attribution::default();
    loop {
        exec.step()
            .unwrap_or_else(|e| panic!("{}: engine execution failed: {e}", program.name));
        // The step that ends the program reports nothing; it executes the
        // root's `END`, a control-flow cycle.
        if exec.stopped {
            break;
        }
        if exec.current_op.is_none_or(|op| matches!(op, Operation::Noop)) {
            continue;
        }
        let Some(asmop) = exec.current_asmop.as_ref() else {
            continue;
        };
        let Some(location) = asmop.location() else { continue };
        let key = location_key(location);
        *attribution.cycles.entry((key.clone(), asmop.num_cycles())).or_default() += 1;
        attribution.rows.entry(key).or_insert_with(|| Row {
            op: asmop.op().to_string(),
            context: asmop.context_name().to_string(),
        });
    }
    attribution
}

/// Runs `program` under the collector and writes the run to `cov`.
fn collect(program: &Program, package: &Arc<Package>, cov: &Path) {
    let mut host = DefaultHost::default();
    host.load_library(HostLibrary::from(CoreLibrary::default().package())).unwrap();
    let processor = FastProcessor::new_with_options(
        program.stack_inputs(),
        program.advice_inputs(),
        ExecutionOptions::default(),
    )
    .unwrap();
    let mut stepper = Stepper::for_package(processor, package.clone()).unwrap();
    let mut collector = Collector::new();
    stepper
        .run_sync(&mut host, |cycle| collector.on_cycle(cycle))
        .unwrap_or_else(|e| panic!("{}: collector execution failed: {e}", program.name));
    let output = collector.finish(program.name.into(), RunOutcome::Ok);
    CoverageDir::new(cov).write_output(&output).unwrap();
}

/// The location and description of `row`, if it has a location.
fn located_row(
    debug_info: &PackageDebugInfo,
    row: &DebugSourceAsmOp,
) -> Option<(LocationKey, Row)> {
    let location = debug_info.get_location(row.location_idx.into_option()?)?;
    let string = |idx| debug_info.get_string(idx).as_deref().unwrap_or("").to_owned();
    Some((
        location_key(&location),
        Row {
            op: string(row.op_name_idx),
            context: string(row.context_name_idx),
        },
    ))
}

/// Our attribution of the runs in the coverage directory `cov`.
fn collected(cov: &Path) -> Collected {
    let snapshot = Snapshot::load(&CoverageDir::new(cov)).unwrap();
    let mut collected = Collected::default();
    for (commitment, package) in &snapshot.packages {
        let Some(hits) = snapshot.merged.get(commitment) else {
            continue;
        };
        let debug_info = &package.debug_info;
        for (node_id, node) in debug_info.nodes().iter().enumerate() {
            let node_id = node_id as u32;
            // Most nodes of a linked library never run.
            if hits.op_hits.range((node_id, 0)..=(node_id, u32::MAX)).next().is_none() {
                continue;
            }
            let op_hits = |op_idx: u32| hits.op_hits.get(&(node_id, op_idx)).copied().unwrap_or(0);

            // Check 2: the reporter's rule, one count per row with operations.
            for row in node.asm_ops.iter().filter(|row| row.num_cycles > 0) {
                let count = op_hits(row.op_idx);
                if count > 0
                    && let Some((key, _)) = located_row(debug_info, row)
                {
                    *collected.executions.entry(key).or_default() += count;
                }
            }

            // Check 1: every operation of the block, charged to the last row
            // at or before it.
            let Some(MastNode::Block(block)) = package.forest.get_node_by_id(node.exec_node) else {
                continue;
            };
            let ops = block.op_batches().iter().flat_map(|batch| batch.ops());
            // Operations charged to the current row so far, padding excluded;
            // those past its `num_cycles` belong to no row.
            let mut row_ops = 0;
            let mut current = None;
            for (op_idx, op) in ops.enumerate() {
                let op_idx = op_idx as u32;
                let row = node.asm_op_for_operation(op_idx);
                let id = row.map(|row| row as *const DebugSourceAsmOp);
                if id != current {
                    current = id;
                    row_ops = 0;
                }
                if matches!(op, Operation::Noop) {
                    continue;
                }
                row_ops += 1;
                let count = op_hits(op_idx);
                let Some(row) = row.filter(|_| count > 0) else { continue };
                let Some((key, description)) = located_row(debug_info, row) else {
                    continue;
                };
                let row_key = (key.clone(), row.num_cycles);
                *collected.attribution.cycles.entry(row_key.clone()).or_default() += count;
                if row_ops > u32::from(row.num_cycles) {
                    *collected.rowless.entry(row_key).or_default() += count;
                }
                collected.attribution.rows.entry(key).or_insert(description);
            }
        }
    }
    collected
}

/// A location for messages: the file, the 1-based line when the file is
/// readable, and the byte range.
fn show((uri, start, end): &LocationKey) -> String {
    let line = fs::read(uri).ok().and_then(|text| {
        let before = text.get(..*start as usize)?;
        Some(before.iter().filter(|byte| **byte == b'\n').count() + 1)
    });
    match line {
        Some(line) => format!("{uri}:{line} [{start}..{end})"),
        None => format!("{uri} [{start}..{end})"),
    }
}

/// Every key where `ours` and `theirs` differ, described with both counts
/// and the row behind the location.
fn diff<K: Ord>(
    (ours_name, ours): (&str, &BTreeMap<K, u64>),
    (theirs_name, theirs): (&str, &BTreeMap<K, u64>),
    describe: impl Fn(&K) -> String,
) -> Vec<String> {
    let keys: BTreeSet<&K> = ours.keys().chain(theirs.keys()).collect();
    keys.into_iter()
        .filter(|key| ours.get(*key) != theirs.get(*key))
        .map(|key| {
            let count = |map: &BTreeMap<K, u64>| map.get(key).copied().unwrap_or(0);
            format!(
                "{}: {ours_name} {}, {theirs_name} {}",
                describe(key),
                count(ours),
                count(theirs)
            )
        })
        .collect()
}

/// A location with its row, for messages.
fn describe_location(rows: &BTreeMap<LocationKey, Row>, key: &LocationKey) -> String {
    match rows.get(key) {
        Some(row) => format!("{} `{}` in {}", show(key), row.op, row.context),
        None => show(key),
    }
}

/// The engine's executions per location, cycles divided by `num_cycles`,
/// and the locations where that division does not measure executions.
fn engine_executions(
    engine: &Attribution,
    ours: &Collected,
) -> (BTreeMap<LocationKey, u64>, BTreeSet<LocationKey>) {
    let mut executions = BTreeMap::new();
    let mut skipped: BTreeSet<LocationKey> =
        ours.rowless.keys().map(|(key, _)| key.clone()).collect();
    for ((key, num_cycles), cycles) in &engine.cycles {
        if *num_cycles == 0 || cycles % u64::from(*num_cycles) != 0 {
            skipped.insert(key.clone());
            continue;
        }
        *executions.entry(key.clone()).or_default() += cycles / u64::from(*num_cycles);
    }
    for key in &skipped {
        executions.remove(key);
    }
    (executions, skipped)
}

/// Runs `program` through the engine and the collector and asserts that both
/// checks agree. Returns our attribution.
fn check(program: &Program) -> Collected {
    let started = Instant::now();
    let package = program.assemble();
    let engine = engine_attribution(program, &package);
    let engine_time = started.elapsed();
    let tmp = tempfile::tempdir().unwrap();
    collect(program, &package, tmp.path());
    let ours = collected(tmp.path());

    let rows = |key: &LocationKey| describe_location(&engine.rows, key);
    let mut problems: Vec<String> = diff(
        ("collector", &ours.attribution.cycles),
        ("engine", &engine.cycles),
        |(key, n)| format!("cycles of {} ({n} per execution)", rows(key)),
    );

    let (engine_executions, skipped) = engine_executions(&engine, &ours);
    let mut our_executions = ours.executions.clone();
    our_executions.retain(|key, _| !skipped.contains(key));
    problems.extend(diff(("collector", &our_executions), ("engine", &engine_executions), |key| {
        format!("executions of {}", rows(key))
    }));

    eprintln!(
        "{}: {} locations, {} skipped by check 2, engine {engine_time:?}, total {:?}",
        program.name,
        ours.executions.len(),
        skipped.len(),
        started.elapsed()
    );
    for ((key, n), cycles) in &ours.rowless {
        eprintln!(
            "  {} ({n} per execution): {cycles} cycles of operations without a row",
            rows(key)
        );
    }
    assert!(
        problems.is_empty(),
        "{}: attribution disagrees:\n  {}",
        program.name,
        problems.join("\n  ")
    );
    assert!(!ours.executions.is_empty(), "{}: nothing was attributed", program.name);
    ours
}

/// The fixture with the flag and loop counter on the stack.
fn fixture(flag: u64, n: u64) -> Program {
    Program::new("fixture", "fixture.masm", vec![flag, n])
}

/// The count of the location starting at the first occurrence of `needle` in
/// the fixture.
fn count_at(counts: &BTreeMap<LocationKey, u64>, needle: &str) -> Option<u64> {
    let source = fs::read_to_string(fixture(0, 0).path()).unwrap();
    let start = source.find(needle).unwrap() as u32;
    let mut matching = counts
        .iter()
        .filter(|((uri, s, _), _)| uri.ends_with("fixture.masm") && *s == start);
    let found = matching.next().map(|(_, count)| *count);
    assert!(matching.next().is_none(), "several locations start at `{needle}`");
    found
}

#[test]
fn oracle_reproduces_known_fixture_counts() {
    let program = fixture(1, 3);
    let engine = engine_attribution(&program, &program.assemble());
    let (executions, skipped) = engine_executions(&engine, &Collected::default());
    assert!(skipped.is_empty(), "{skipped:?}");
    // `double` runs once; the loop body runs n = 3 doubled = 6 times.
    assert_eq!(count_at(&executions, "mul.2"), Some(1));
    assert_eq!(count_at(&executions, "sub.1"), Some(6));
    // The else arm is not taken, and the guard's zero-op row has no
    // operations for the engine to report.
    assert_eq!(count_at(&executions, "push.7"), None);
    assert_eq!(count_at(&executions, "while.true"), None);
}

#[test]
fn fixture_agrees_with_engine() {
    check(&fixture(1, 3));
    check(&fixture(0, 0));
}

#[test]
fn fib_agrees_with_engine() {
    check(&Program::new("fib", "examples/fib.masm", vec![1]));
}

#[test]
fn nprime_agrees_with_engine() {
    check(&Program::new("nprime", "examples/nprime.masm", vec![50]));
}

#[test]
fn blake3_1to1_agrees_with_engine() {
    check(
        &Program::new("blake3_1to1", "examples/blake3_1to1.masm", vec![u32::MAX.into(); 8])
            .with_advice(vec![100]),
    );
}

#[test]
fn blake3_2to1_agrees_with_engine() {
    check(&Program::new(
        "blake3_2to1",
        "examples/blake3_2to1.masm",
        vec![u32::MAX.into(); 16],
    ));
}

#[test]
fn sha256_agrees_with_engine() {
    check(&Program::new("sha256", "examples/sha256.masm", vec![u32::MAX.into(); 16]));
}

/// Runs `program` through the `miden-cov run` binary into `cov`, linking the
/// core library from a package file.
fn cli_run(program: &Program, dir: &Path, cov: &Path) {
    let masp = dir.join(format!("{}.masp", program.name));
    program.assemble().write_to_file(&masp).unwrap();
    let core = dir.join("miden-core.masp");
    CoreLibrary::default().package().write_to_file(&core).unwrap();
    let list = |values: &[u64]| values.iter().map(u64::to_string).collect::<Vec<_>>().join(", ");
    let inputs = dir.join(format!("{}.toml", program.name));
    fs::write(
        &inputs,
        format!(
            "[inputs]\nstack = [{}]\n\n[inputs.advice]\nstack = [{}]\n",
            list(&program.stack),
            list(&program.advice)
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_miden-cov"))
        .arg("run")
        .arg(&masp)
        .arg("--inputs")
        .arg(&inputs)
        .arg("-l")
        .arg(&core)
        .arg("--cov-dir")
        .arg(cov)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}: stdout: {}\nstderr: {}",
        program.name,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_run_agrees_with_collector() {
    for program in [
        fixture(1, 3),
        Program::new("nprime", "examples/nprime.masm", vec![50]),
        Program::new("blake3_2to1", "examples/blake3_2to1.masm", vec![u32::MAX.into(); 16]),
    ] {
        let direct = check(&program);
        let tmp = tempfile::tempdir().unwrap();
        let cov = tmp.path().join("cov");
        cli_run(&program, tmp.path(), &cov);
        let cli = collected(&cov);
        let rows = |key: &LocationKey| describe_location(&direct.attribution.rows, key);
        let mut problems =
            diff(("cli", &cli.executions), ("collector", &direct.executions), |key| {
                format!("executions of {}", rows(key))
            });
        problems.extend(diff(
            ("cli", &cli.attribution.cycles),
            ("collector", &direct.attribution.cycles),
            |(key, n)| format!("cycles of {} ({n} per execution)", rows(key)),
        ));
        assert!(
            problems.is_empty(),
            "{}: the CLI and the collector disagree:\n  {}",
            program.name,
            problems.join("\n  ")
        );
    }
}
