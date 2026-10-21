//! A program executor that records the coverage of what it runs.

use alloc::sync::Arc;

use miden_debug_engine::Stepper;
use miden_mast_package::debug_info::{DebugSourceNodeId, PackageDebugInfo};
use miden_processor::{
    ExecutionError, ExecutionOptions, ExecutionOutput, FastProcessor, FutureMaybeSend, Host,
    Program, ProgramExecutor, StackInputs,
    advice::{AdviceError, AdviceInputs},
};

use crate::collector::Collector;

/// Executes a program and records its coverage when a coverage directory is
/// configured.
///
/// Plugs into anything generic over [`ProgramExecutor`], such as the
/// protocol's transaction executor, with any host. When the coverage
/// directory environment variable is set and the program comes with debug
/// info, the program runs through the engine's stepper and each execution
/// writes one run file named after the thread that started it, which under
/// the standard test runner is the test; otherwise, and without `std`, the
/// program runs through the processor's own [`ProgramExecutor`] and nothing
/// changes. Coverage never changes the
/// outcome of an execution: a run file that cannot be written is reported on
/// standard error and the outputs are returned as usual.
///
/// The step API cannot start from a given entry-point source node; it starts
/// from the unique source root of the entry point. When an entry-point node
/// is supplied with debug info and that root is not the same node, the
/// program runs through the processor's own executor without collecting, and
/// says so on standard error.
pub struct CoverageExecutor {
    /// The processor configured with the inputs and the options.
    processor: FastProcessor,
    /// The package debug info to attribute the execution to, if any.
    debug_info: Option<PackageDebugInfo>,
    /// The source node to start at, if the caller named one.
    entrypoint: Option<DebugSourceNodeId>,
}

impl ProgramExecutor for CoverageExecutor {
    fn new(
        stack_inputs: StackInputs,
        advice_inputs: AdviceInputs,
        options: ExecutionOptions,
    ) -> Result<Self, AdviceError> {
        Ok(Self {
            processor: FastProcessor::new_with_options(stack_inputs, advice_inputs, options)?,
            debug_info: None,
            entrypoint: None,
        })
    }

    fn with_debug_info(mut self, package_debug_info: PackageDebugInfo) -> Self {
        self.debug_info = Some(package_debug_info);
        self
    }

    fn with_entrypoint_source_node(
        mut self,
        entrypoint_source_node: Option<DebugSourceNodeId>,
    ) -> Self {
        self.entrypoint = entrypoint_source_node;
        self
    }

    fn execute<H: Host + Send>(
        self,
        program: &Program,
        host: &mut H,
    ) -> impl FutureMaybeSend<Result<ExecutionOutput, ExecutionError>> {
        // The run is named after the thread that starts the execution, not
        // after whichever thread polls the future to its end.
        let run = coverage_run();
        async move {
            let Self { processor, debug_info, entrypoint } = self;
            match (run, debug_info) {
                // Stepping needs debug info to attribute anything to; a program
                // without any is run as if coverage were off.
                (Some(run), Some(debug_info))
                    if startable_by_stepping(program, &debug_info, entrypoint) =>
                {
                    let mut collector = Collector::new();
                    let result = async {
                        // A context that cannot be built is an execution that
                        // failed before its first cycle; it is written like any
                        // other.
                        let mut processor = processor;
                        let ctx = processor.get_initial_resume_context(program)?;
                        let mut stepper =
                            Stepper::new(processor, ctx).with_debug_info(Arc::new(debug_info));
                        stepper.run(host, |cycle| collector.on_cycle(cycle)).await?;
                        stepper.into_output()
                    }
                    .await;
                    // The run is written before the error is returned, so a
                    // failing test still reports what it executed.
                    write_run(run, collector, &result);
                    result
                },
                (_, debug_info) => {
                    // The processor's own executor. Its inherent `execute`
                    // shadows the trait's, and only the trait's reads the debug
                    // info and the entry point it was given.
                    let mut processor = processor;
                    if let Some(debug_info) = debug_info {
                        processor = processor.with_debug_info(debug_info);
                    }
                    processor = processor.with_entrypoint_source_node(entrypoint);
                    <FastProcessor as ProgramExecutor>::execute(processor, program, host).await
                },
            }
        }
    }
}

/// Whether stepping from the program's unique source root starts where the
/// caller asked: at the named entry point, or at whatever the processor's own
/// executor would start at when none was named, which also fails on an
/// ambiguous root, before any cycle, the way that executor does.
///
/// Says on standard error when it does not, since the execution then runs
/// without coverage.
fn startable_by_stepping(
    program: &Program,
    debug_info: &PackageDebugInfo,
    entrypoint: Option<DebugSourceNodeId>,
) -> bool {
    let root = debug_info.unique_source_root_for_exec_node(program.entrypoint());
    let startable = match entrypoint {
        Some(entrypoint) => root == Ok(Some(entrypoint)),
        None => root.is_ok(),
    };
    if startable {
        return true;
    }
    #[cfg(feature = "std")]
    std::eprintln!(
        "miden-cov: the entry-point source node cannot be reached by stepping; running without \
         coverage"
    );
    false
}

/// The coverage directory and the name of the run to write, when the
/// directory is configured.
#[cfg(feature = "std")]
struct CoverageRun {
    /// Where the run is written.
    dir: crate::dir::CoverageDir,
    /// The name of the thread that started the execution.
    name: alloc::string::String,
}

/// The run to write, when the coverage directory is configured.
#[cfg(feature = "std")]
fn coverage_run() -> Option<CoverageRun> {
    use alloc::string::ToString;

    let dir = crate::dir::CoverageDir::from_env()?;
    let name = std::thread::current().name().unwrap_or("unnamed").to_string();
    Some(CoverageRun { dir, name })
}

/// Writes the run to the coverage directory it was configured with.
#[cfg(feature = "std")]
fn write_run(
    run: CoverageRun,
    collector: Collector,
    result: &Result<ExecutionOutput, ExecutionError>,
) {
    use crate::format::RunOutcome;

    let output = collector.finish(run.name, RunOutcome::of(result));
    if let Err(error) = run.dir.write_output(&output) {
        std::eprintln!(
            "miden-cov: failed to write run '{}' to '{}': {error}",
            output.run.name,
            run.dir.root().display()
        );
    }
}

/// Without `std` there is no coverage directory, so there is never a run.
#[cfg(not(feature = "std"))]
enum CoverageRun {}

/// Without `std` there is no coverage directory, so nothing is collected.
#[cfg(not(feature = "std"))]
fn coverage_run() -> Option<CoverageRun> {
    None
}

/// Without `std` nothing is collected, so there is nothing to write.
#[cfg(not(feature = "std"))]
fn write_run(
    _run: CoverageRun,
    _collector: Collector,
    _result: &Result<ExecutionOutput, ExecutionError>,
) {
}
