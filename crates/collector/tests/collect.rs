//! Collects a small MASM program through the engine's stepper and checks the
//! raw hits, the coverage directory round trip, and the program executor.

use std::{
    pin::pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use miden_assembly::Assembler;
use miden_core::{Felt, mast::MastNode};
use miden_cov_collector::{
    Collector, CoverageExecutor, PackageHits, RunOutcome,
    dir::{COVERAGE_DIR_ENV, CoverageDir},
};
use miden_debug_engine::Stepper;
use miden_mast_package::Package;
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

fn fixture() -> Arc<Package> {
    Arc::from(Assembler::default().assemble_program("fixture", SRC).unwrap())
}

/// Runs the fixture with `flag` and `n` on the stack and returns the hits of
/// its only package.
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

/// Finds the source node whose exec node is the only node of the given kind
/// with the given number of children sources, by matching a predicate.
fn source_node(package: &Package, pred: impl Fn(&MastNode) -> bool) -> Vec<u32> {
    let debug_info = package.debug_info().unwrap().unwrap();
    let forest = package.mast_forest();
    debug_info
        .nodes()
        .iter()
        .enumerate()
        .filter(|(_, node)| pred(&forest[node.exec_node]))
        .map(|(i, _)| i as u32)
        .collect()
}

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
/// way the protocol's transaction executor drives it.
fn execute(package: &Package, flag: u64, n: u64) -> Result<ExecutionOutput, ExecutionError> {
    let stack = StackInputs::new(&[Felt::new(flag).unwrap(), Felt::new(n).unwrap()]).unwrap();
    let executor = <CoverageExecutor as ProgramExecutor>::new(
        stack,
        AdviceInputs::default(),
        ExecutionOptions::default(),
    )
    .unwrap()
    .with_debug_info(package.debug_info().unwrap().unwrap());
    block_on(executor.execute(&package.unwrap_program(), &mut DefaultHost::default()))
}

/// Points the coverage directory environment variable at `dir`.
fn set_coverage_dir(dir: &std::path::Path) {
    // SAFETY: the suite runs under nextest, one process per test, so the
    // process-global variable is private to this test and no other thread
    // reads the environment concurrently.
    unsafe { std::env::set_var(COVERAGE_DIR_ENV, dir) };
}

#[test]
fn coverage_executor_runs_with_a_default_host() {
    let package = fixture();
    let thread_name = std::thread::current().name().unwrap_or("unnamed").to_owned();

    let tmp = tempfile::tempdir().unwrap();
    set_coverage_dir(tmp.path());
    let output = execute(&package, 1, 3).unwrap();
    let expected = FastProcessor::new(
        StackInputs::new(&[Felt::new(1).unwrap(), Felt::new(3).unwrap()]).unwrap(),
    )
    .execute_sync(&package.unwrap_program(), &mut DefaultHost::default())
    .unwrap();
    assert_eq!(output.stack, expected.stack);

    let dir = CoverageDir::new(tmp.path());
    let runs = dir.read_runs().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].name, thread_name);
    assert_eq!(runs[0].outcome, RunOutcome::Ok);
    assert_eq!(runs[0].packages.len(), 1);
    let (commitment, hits) = &runs[0].packages[0];
    assert_eq!(*commitment, package.mast_forest_commitment());
    assert!(!hits.is_empty());
    let packages = dir.read_packages().unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].commitment, package.mast_forest_commitment());

    // A flag that is not binary fails the `if.true`; the run is still written.
    let tmp = tempfile::tempdir().unwrap();
    set_coverage_dir(tmp.path());
    execute(&package, 2, 3).unwrap_err();
    let runs = CoverageDir::new(tmp.path()).read_runs().unwrap();
    assert_eq!(runs.len(), 1);
    assert!(matches!(runs[0].outcome, RunOutcome::Failed(_)), "{:?}", runs[0].outcome);
}
