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
/// directory environment variable is set, the program runs through the
/// engine's stepper and each execution writes one run file named after the
/// thread that started it, which under the standard test runner is the test;
/// otherwise, and without `std`, the program runs through the processor's
/// own [`ProgramExecutor`] and nothing changes. Coverage never changes the
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
            let Self { mut processor, debug_info, entrypoint } = self;
            let collect = run.filter(|_| {
                debug_info.is_some() && startable_by_stepping(program, &debug_info, entrypoint)
            });
            let Some(run) = collect else {
                // The processor's own executor, with the inherent `execute`
                // shadowed by the trait's: only the trait's reads the debug
                // info and the entry point it was given.
                if let Some(debug_info) = debug_info {
                    processor = processor.with_debug_info(debug_info);
                }
                processor = processor.with_entrypoint_source_node(entrypoint);
                return <FastProcessor as ProgramExecutor>::execute(processor, program, host).await;
            };
            let mut collector = Collector::new();
            // A context that cannot be built is an execution that failed
            // before its first cycle; it is written like any other.
            let result = match processor.get_initial_resume_context(program) {
                Ok(ctx) => {
                    let mut stepper = Stepper::new(processor, ctx);
                    if let Some(debug_info) = debug_info {
                        stepper = stepper.with_debug_info(Arc::new(debug_info));
                    }
                    let result = stepper.run(host, |cycle| collector.on_cycle(cycle)).await;
                    result.and_then(|()| stepper.into_output())
                },
                Err(error) => Err(error),
            };
            // The run is written before the error is returned, so a failing
            // test still reports what it executed.
            write_run(run, collector, result.as_ref().map(|_| ()));
            result
        }
    }
}

/// Whether stepping from the program's unique source root starts where the
/// caller asked, which is always the case when no entry point was named.
///
/// Says on standard error when it does not, since the execution then runs
/// without coverage.
fn startable_by_stepping(
    program: &Program,
    debug_info: &Option<PackageDebugInfo>,
    entrypoint: Option<DebugSourceNodeId>,
) -> bool {
    let Some(entrypoint) = entrypoint else {
        return true;
    };
    let root = debug_info
        .as_ref()
        .and_then(|info| info.unique_source_root_for_exec_node(program.entrypoint()).ok())
        .flatten();
    if root == Some(entrypoint) {
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
    dir: crate::dir::CoverageDir,
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
fn write_run(run: CoverageRun, collector: Collector, result: Result<(), &ExecutionError>) {
    use crate::format::RunOutcome;

    let output = collector.finish(run.name, RunOutcome::of(&result));
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
fn write_run(_run: CoverageRun, _collector: Collector, _result: Result<(), &ExecutionError>) {}
