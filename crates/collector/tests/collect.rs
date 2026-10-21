//! Collects a small MASM program through the engine's stepper and checks the
//! raw hits, the coverage directory round trip, and the program executor.

use std::{
    pin::pin,
    process::Command,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use miden_assembly::Assembler;
use miden_core::{Felt, mast::MastNode};
use miden_cov_collector::{
    Collector, CoverageExecutor, PackageHits, RunOutcome, Stepper,
    dir::{COVERAGE_DIR_ENV, CoverageDir},
};
use miden_mast_package::{Package, QualifiedProcedureName, debug_info::DebugSourceNodeId};
use miden_processor::{
    DefaultHost, ExecutionError, ExecutionOptions, ExecutionOutput, FastProcessor, ProgramExecutor,
    StackInputs, advice::AdviceInputs,
};

/// Two procedures, an if/else, and a loop guarded by the split the assembler
/// wraps around every `while.true`.
const SRC: &str = r#"
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
"#;

/// The fixture, assembled from the source above.
fn fixture() -> Arc<Package> {
    Arc::from(Assembler::default().assemble_program("fixture", SRC).unwrap())
}

/// Runs the fixture with `flag` and `n` on the stack and returns the hits of
/// its only package and the number of cycles recorded.
fn run(package: &Arc<Package>, flag: u64, n: u64) -> (PackageHits, u64) {
    let mut host = DefaultHost::default();
    let processor = FastProcessor::new(
        StackInputs::new(&[Felt::new(flag).unwrap(), Felt::new(n).unwrap()]).unwrap(),
    );
    let mut stepper = Stepper::for_package(processor, package.clone()).unwrap();
    let mut collector = Collector::new();
    stepper.run_sync(&mut host, |cycle| collector.on_cycle(cycle)).unwrap();
    let output = collector.finish("test".into(), RunOutcome::Ok);
    assert_eq!(output.packages.len(), 1);
    assert_eq!(output.packages[0].commitment, package.mast_forest_commitment());
    let (commitment, hits) = output.run.packages.into_iter().next().unwrap();
    assert_eq!(commitment, package.mast_forest_commitment());
    (hits, output.run.cycles)
}

/// The source nodes of the entry point's own tree whose exec node satisfies
/// `pred`, in id order.
fn source_node(package: &Package, pred: impl Fn(&MastNode) -> bool) -> Vec<u32> {
    let debug_info = package.debug_info().unwrap().unwrap();
    let forest = package.mast_forest();
    // The assembler emits a source tree per procedure and one per call site;
    // only the entry point's tree executes.
    let root = debug_info
        .unique_source_root_for_exec_node(package.unwrap_program().entrypoint())
        .unwrap()
        .unwrap();
    let mut found = Vec::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        let node = &debug_info[id];
        pending.extend(node.children.iter().copied());
        if pred(&forest[node.exec_node]) {
            found.push(u32::from(id));
        }
    }
    found.sort_unstable();
    found
}

/// Node, edge, and operation hits of the fixture for a run that takes the `if`
/// and loops, and one that takes the `else` and skips the loop.
#[test]
fn hits_follow_the_control_flow() {
    let package = fixture();
    let debug_info = package.debug_info().unwrap().unwrap();
    let splits = source_node(&package, |n| matches!(n, MastNode::Split(_)));
    let loops = source_node(&package, |n| matches!(n, MastNode::Loop(_)));
    assert_eq!(loops.len(), 1);
    assert_eq!(splits.len(), 2);
    let loop_src = loops[0];
    let body_src: u32 = debug_info.nodes()[loop_src.into()].children[0].into();
    // The split guarding the loop has the loop as its true child.
    let loop_guard = *splits
        .iter()
        .find(|s| u32::from(debug_info.nodes()[(**s).into()].children[0]) == loop_src)
        .unwrap();
    let if_split = *splits.iter().find(|s| **s != loop_guard).unwrap();
    let if_node = &debug_info.nodes()[if_split.into()];
    let (then_src, else_src): (u32, u32) = (if_node.children[0].into(), if_node.children[1].into());
    let guard_node = &debug_info.nodes()[loop_guard.into()];
    let skip_src: u32 = guard_node.children[1].into();

    // flag = 1 doubles n = 3, so the loop body runs six times.
    let (hits, cycles) = run(&package, 1, 3);
    assert!(cycles > 0);
    // Each start is also recorded as an edge from the node that started it.
    assert_eq!(hits.edge_starts.get(&(if_split, then_src)), Some(&1));
    assert_eq!(hits.edge_starts.get(&(if_split, else_src)), None);
    assert_eq!(hits.edge_starts.get(&(loop_guard, loop_src)), Some(&1));
    assert_eq!(hits.edge_starts.get(&(loop_src, body_src)), Some(&6));
    assert_eq!(hits.node_starts.get(&then_src), Some(&1));
    assert_eq!(hits.node_starts.get(&else_src), None);
    assert_eq!(hits.node_starts.get(&loop_src), Some(&1));
    assert_eq!(hits.node_starts.get(&body_src), Some(&6));
    assert_eq!(hits.node_starts.get(&skip_src), None);
    // `mul.2` is the first row of the `double` block: push(2), mul.
    assert_eq!(hits.op_hits.get(&(then_src, 0)), Some(&1));
    assert_eq!(hits.op_hits.get(&(then_src, 1)), Some(&1));
    // The body block has five operations, each executed once per iteration.
    for op in 0..5 {
        assert_eq!(hits.op_hits.get(&(body_src, op)), Some(&6), "op {op}");
    }

    // flag = 0 takes the else arm and n = 0 skips the loop entirely.
    let (hits, _) = run(&package, 0, 0);
    assert_eq!(hits.edge_starts.get(&(if_split, else_src)), Some(&1));
    assert_eq!(hits.edge_starts.get(&(loop_guard, skip_src)), Some(&1));
    assert_eq!(hits.edge_starts.get(&(loop_src, body_src)), None);
    assert_eq!(hits.node_starts.get(&then_src), None);
    assert_eq!(hits.node_starts.get(&else_src), Some(&1));
    assert_eq!(hits.node_starts.get(&loop_src), None);
    assert_eq!(hits.node_starts.get(&body_src), None);
    assert_eq!(hits.node_starts.get(&skip_src), Some(&1));
}

/// A run and its package survive the trip through the coverage directory.
#[test]
fn coverage_dir_round_trip() {
    let package = fixture();
    let tmp = tempfile::tempdir().unwrap();
    let dir = CoverageDir::new(tmp.path().join("cov"));

    let mut host = DefaultHost::default();
    let processor = FastProcessor::new(
        StackInputs::new(&[Felt::new(1).unwrap(), Felt::new(2).unwrap()]).unwrap(),
    );
    let mut stepper = Stepper::for_package(processor, package.clone()).unwrap();
    let mut collector = Collector::new();
    stepper.run_sync(&mut host, |cycle| collector.on_cycle(cycle)).unwrap();
    let output = collector.finish("round-trip".into(), RunOutcome::Failed("boom".into()));
    let run_path = dir.write_output(&output).unwrap();
    // Writing the same package again is a no-op.
    dir.write_package(&output.packages[0]).unwrap();

    let packages = dir.read_packages().unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].commitment, package.mast_forest_commitment());
    assert_eq!(packages[0].forest.commitment(), package.mast_forest().commitment());
    assert_eq!(
        packages[0].debug_info.nodes().len(),
        output.packages[0].debug_info.nodes().len()
    );

    let runs = dir.read_runs().unwrap();
    assert_eq!(runs.len(), 1);
    let run = CoverageDir::read_run(&run_path).unwrap();
    assert_eq!(run.name, "round-trip");
    assert_eq!(run.outcome, RunOutcome::Failed("boom".into()));
    assert_eq!(run.cycles, output.run.cycles);
    assert_eq!(run.packages, output.run.packages);
    assert_eq!(runs[0].packages, output.run.packages);
}

/// Polls `future` to completion on the current thread.
///
/// The executor's future never waits on anything with a default host, so a
/// no-op waker is enough.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
    }
}

/// Executes the fixture with `flag` and `n` through [`CoverageExecutor`], the
/// way the protocol's transaction executor drives it, starting at
/// `entrypoint` when one is given.
fn execute(
    package: &Package,
    flag: u64,
    n: u64,
    entrypoint: Option<DebugSourceNodeId>,
) -> Result<ExecutionOutput, ExecutionError> {
    let stack = StackInputs::new(&[Felt::new(flag).unwrap(), Felt::new(n).unwrap()]).unwrap();
    let executor = <CoverageExecutor as ProgramExecutor>::new(
        stack,
        AdviceInputs::default(),
        ExecutionOptions::default(),
    )
    .unwrap()
    .with_debug_info(package.debug_info().unwrap().unwrap())
    .with_entrypoint_source_node(entrypoint);
    block_on(executor.execute(&package.unwrap_program(), &mut DefaultHost::default()))
}

/// The outputs of the fixture from the processor's own program executor,
/// with the same debug info and entry point.
fn expected_output(
    package: &Package,
    flag: u64,
    n: u64,
    entrypoint: Option<DebugSourceNodeId>,
) -> Result<ExecutionOutput, ExecutionError> {
    let stack = StackInputs::new(&[Felt::new(flag).unwrap(), Felt::new(n).unwrap()]).unwrap();
    let processor = <FastProcessor as ProgramExecutor>::new(
        stack,
        AdviceInputs::default(),
        ExecutionOptions::default(),
    )
    .unwrap()
    .with_debug_info(package.debug_info().unwrap().unwrap())
    .with_entrypoint_source_node(entrypoint);
    block_on(<FastProcessor as ProgramExecutor>::execute(
        processor,
        &package.unwrap_program(),
        &mut DefaultHost::default(),
    ))
}

/// A program whose outputs are not all zero, so a reversed or dropped stack
/// shows: it leaves 11 on top of the stack.
fn sum_program() -> Arc<Package> {
    Arc::from(
        Assembler::default()
            .assemble_program("sum", "begin push.5 push.6 add swap drop end")
            .unwrap(),
    )
}

/// Runs the ignored test `name` of this binary in a child process started in
/// `cwd`, with the coverage directory variable set to `dir` or removed.
///
/// The variable is process-global and reading it from another thread while it
/// changes is undefined behavior, so it is only ever set for a child process.
fn run_in_child(name: &str, cwd: &std::path::Path, dir: Option<&std::path::Path>) -> String {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", name, "--ignored", "--nocapture"]).current_dir(cwd);
    match dir {
        Some(dir) => command.env(COVERAGE_DIR_ENV, dir),
        None => command.env_remove(COVERAGE_DIR_ENV),
    };
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success()
            && stdout.contains("test result: ok. 1 passed")
            && !stderr.contains("skipped:"),
        "{name} did not run once and pass:\n{stdout}\n{stderr}"
    );
    stderr.into_owned()
}

/// The body of the child process with the coverage directory set: one run
/// per execution, whatever the outcome, and none when the entry point cannot
/// be reached by stepping.
#[test]
#[ignore = "run by coverage_executor_writes_one_run_per_execution in a child process"]
fn coverage_executor_child_with_directory() {
    if CoverageDir::from_env().is_none() {
        eprintln!("skipped: the parent test sets the coverage directory");
        return;
    }
    let package = fixture();
    let output = execute(&package, 1, 3, None).unwrap();
    assert_eq!(output.stack, expected_output(&package, 1, 3, None).unwrap().stack);
    // A flag that is not binary fails the `if.true` with the processor's error.
    let ours = execute(&package, 2, 3, None).unwrap_err().to_string();
    let theirs = expected_output(&package, 2, 3, None).unwrap_err().to_string();
    assert_eq!(ours, theirs);
    // The entry point's unique source root is where stepping starts anyway.
    let debug_info = package.debug_info().unwrap().unwrap();
    let root = debug_info
        .unique_source_root_for_exec_node(package.unwrap_program().entrypoint())
        .unwrap()
        .unwrap();
    execute(&package, 1, 3, Some(root)).unwrap();
    // Any other node cannot be reached by stepping: the program goes to the
    // processor's own executor, which rejects a node that is not the entry's,
    // and nothing is written.
    let other = DebugSourceNodeId::from(if u32::from(root) == 0 { 1 } else { 0 });
    let ours = execute(&package, 1, 3, Some(other)).unwrap_err().to_string();
    let theirs = expected_output(&package, 1, 3, Some(other)).unwrap_err().to_string();
    assert_eq!(ours, theirs);
}

/// The body of the child process without the coverage directory: the program
/// runs through the plain processor and the outputs are the processor's.
#[test]
#[ignore = "run by coverage_executor_writes_one_run_per_execution in a child process"]
fn coverage_executor_child_without_directory() {
    if CoverageDir::from_env().is_some() {
        eprintln!("skipped: the parent test removes the coverage directory");
        return;
    }
    let package = fixture();
    let output = execute(&package, 1, 3, None).unwrap();
    let expected = expected_output(&package, 1, 3, None).unwrap();
    assert_eq!(output.stack, expected.stack);
    let ours = execute(&package, 2, 3, None).unwrap_err().to_string();
    let theirs = expected_output(&package, 2, 3, None).unwrap_err().to_string();
    assert_eq!(ours, theirs);
    // An error whose message depends on the debug info reaching the
    // processor: the inherent `execute` would lose the `err` of the `assert`.
    let failing: Arc<Package> = Arc::from(
        Assembler::default()
            .assemble_program("failing", "begin assert.err=\"boom\" end")
            .unwrap(),
    );
    let executor = <CoverageExecutor as ProgramExecutor>::new(
        StackInputs::default(),
        AdviceInputs::default(),
        ExecutionOptions::default(),
    )
    .unwrap()
    .with_debug_info(failing.debug_info().unwrap().unwrap());
    let ours = block_on(executor.execute(&failing.unwrap_program(), &mut DefaultHost::default()))
        .unwrap_err()
        .to_string();
    assert!(ours.contains("boom"), "{ours}");
    // A program with a non-trivial stack.
    let sum = sum_program();
    let executor = <CoverageExecutor as ProgramExecutor>::new(
        StackInputs::default(),
        AdviceInputs::default(),
        ExecutionOptions::default(),
    )
    .unwrap()
    .with_debug_info(sum.debug_info().unwrap().unwrap());
    let output =
        block_on(executor.execute(&sum.unwrap_program(), &mut DefaultHost::default())).unwrap();
    assert_eq!(output.stack.get_element(0), Some(Felt::new(11).unwrap()));
}

/// The body of the child process for a program whose outputs are not all
/// zero, with the coverage directory set: the stepper's outputs are the
/// processor's. Then the two cases that go to the processor's own executor
/// with the directory set: no debug info, and an entry point with no unique
/// source root; neither writes a run.
#[test]
#[ignore = "run by coverage_executor_writes_one_run_per_execution in a child process"]
fn coverage_executor_child_outputs() {
    if CoverageDir::from_env().is_none() {
        eprintln!("skipped: the parent test sets the coverage directory");
        return;
    }
    let sum = sum_program();
    let executor = <CoverageExecutor as ProgramExecutor>::new(
        StackInputs::default(),
        AdviceInputs::default(),
        ExecutionOptions::default(),
    )
    .unwrap()
    .with_debug_info(sum.debug_info().unwrap().unwrap());
    let output =
        block_on(executor.execute(&sum.unwrap_program(), &mut DefaultHost::default())).unwrap();
    let expected = FastProcessor::new(StackInputs::default())
        .execute_sync(&sum.unwrap_program(), &mut DefaultHost::default())
        .unwrap();
    assert_eq!(output.stack, expected.stack);
    assert_eq!(output.stack.get_element(0), Some(Felt::new(11).unwrap()));
    assert_eq!(output.precompile_witness.is_some(), expected.precompile_witness.is_some());

    // No debug info: nothing to attribute to, so the processor's own executor
    // runs and the stack is still the program's.
    let executor = <CoverageExecutor as ProgramExecutor>::new(
        StackInputs::default(),
        AdviceInputs::default(),
        ExecutionOptions::default(),
    )
    .unwrap();
    let output =
        block_on(executor.execute(&sum.unwrap_program(), &mut DefaultHost::default())).unwrap();
    assert_eq!(output.stack, expected.stack);

    // An entry point whose source root is ambiguous: a library procedure made
    // executable whose body is one `exec` compiles to the same MAST as its
    // callee, so the entry's exec node has two source roots. The processor's
    // own executor refuses it before any cycle, and so does the coverage
    // executor, with the same error.
    let library = Assembler::default()
        .assemble_library(
            "amb",
            "namespace amb::m pub proc leaf push.1 drop end pub proc middle exec.leaf end pub \
             proc entry exec.middle end",
            None::<String>,
        )
        .unwrap();
    let entrypoint: QualifiedProcedureName = "amb::m::entry".parse().unwrap();
    let ambiguous = Arc::new(library.make_executable(&entrypoint).unwrap());
    let debug_info = ambiguous.debug_info().unwrap().unwrap();
    assert!(
        debug_info
            .unique_source_root_for_exec_node(ambiguous.unwrap_program().entrypoint())
            .is_err(),
        "the fixture's entry root is ambiguous"
    );
    let ours = <CoverageExecutor as ProgramExecutor>::new(
        StackInputs::default(),
        AdviceInputs::default(),
        ExecutionOptions::default(),
    )
    .unwrap()
    .with_debug_info(debug_info.clone());
    let ours = block_on(ours.execute(&ambiguous.unwrap_program(), &mut DefaultHost::default()))
        .unwrap_err()
        .to_string();
    let theirs = <FastProcessor as ProgramExecutor>::new(
        StackInputs::default(),
        AdviceInputs::default(),
        ExecutionOptions::default(),
    )
    .unwrap()
    .with_debug_info(debug_info);
    let theirs = block_on(<FastProcessor as ProgramExecutor>::execute(
        theirs,
        &ambiguous.unwrap_program(),
        &mut DefaultHost::default(),
    ))
    .unwrap_err()
    .to_string();
    assert_eq!(ours, theirs);
}

/// One run per execution with the directory set, none without it, and the
/// same hits as the package entry collects.
#[test]
fn coverage_executor_writes_one_run_per_execution() {
    let package = fixture();
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let notes =
        run_in_child("coverage_executor_child_with_directory", cwd.path(), Some(tmp.path()));
    assert!(notes.contains("cannot be reached by stepping"), "{notes}");
    assert_eq!(std::fs::read_dir(cwd.path()).unwrap().count(), 0, "nothing lands in the cwd");

    let dir = CoverageDir::new(tmp.path());
    let runs = dir.read_runs().unwrap();
    assert_eq!(runs.len(), 3, "{runs:?}");
    assert!(runs.iter().all(|run| run.name == "coverage_executor_child_with_directory"));
    let failed: Vec<_> =
        runs.iter().filter(|run| matches!(run.outcome, RunOutcome::Failed(_))).collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].packages.len(), 1);
    // The two successful runs hold exactly what the package entry collects.
    let (expected_hits, expected_cycles) = run(&package, 1, 3);
    for run in runs.iter().filter(|run| run.outcome == RunOutcome::Ok) {
        assert_eq!(run.cycles, expected_cycles);
        assert_eq!(run.packages.len(), 1);
        assert_eq!(run.packages[0].0, package.mast_forest_commitment());
        assert_eq!(run.packages[0].1, expected_hits);
    }
    let packages = dir.read_packages().unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].commitment, package.mast_forest_commitment());

    let outputs = tempfile::tempdir().unwrap();
    let notes = run_in_child("coverage_executor_child_outputs", cwd.path(), Some(outputs.path()));
    assert!(notes.contains("no unique source root"), "{notes}");
    assert_eq!(CoverageDir::new(outputs.path()).read_runs().unwrap().len(), 1);

    // Without the directory the child runs from an empty directory, where the
    // default coverage directory would be created if anything were written.
    let empty = tempfile::tempdir().unwrap();
    run_in_child("coverage_executor_child_without_directory", empty.path(), None);
    assert_eq!(std::fs::read_dir(empty.path()).unwrap().count(), 0);
}

/// The collector counts one record per VM cycle: the finalization step after
/// the last cycle adds none.
#[test]
fn cycles_match_the_vm_clock() {
    let package = fixture();
    let processor = FastProcessor::new(
        StackInputs::new(&[Felt::new(1).unwrap(), Felt::new(3).unwrap()]).unwrap(),
    );
    let mut stepper = Stepper::for_package(processor, package.clone()).unwrap();
    let mut collector = Collector::new();
    stepper
        .run_sync(&mut DefaultHost::default(), |cycle| collector.on_cycle(cycle))
        .unwrap();
    let clock = u64::from(u32::from(stepper.processor().state().clock()));
    assert_eq!(collector.cycles(), clock);
    assert_eq!(collector.finish("clock".into(), RunOutcome::Ok).run.cycles, clock);
}
