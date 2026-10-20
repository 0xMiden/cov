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

/// Executes a program through the engine's stepper and records its coverage.
///
/// Plugs into anything generic over [`ProgramExecutor`], such as the
/// protocol's transaction executor, with any host. When the coverage
/// directory environment variable is set, each execution writes one run file
/// named after the current thread, which under the standard test runner is
/// the test name; otherwise nothing is written. Coverage never changes the
/// outcome of an execution: a run file that cannot be written is reported on
/// standard error and the outputs are returned as usual.
///
/// The entry-point source node is not honored: the step API has no way to
/// start from a given source occurrence, so execution starts from the
/// program's unique source root, as the debugger does.
pub struct CoverageExecutor {
    processor: FastProcessor,
    debug_info: Option<Arc<PackageDebugInfo>>,
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
        })
    }

    fn with_debug_info(mut self, package_debug_info: PackageDebugInfo) -> Self {
        self.debug_info = Some(Arc::new(package_debug_info));
        self
    }

    fn with_entrypoint_source_node(
        self,
        _entrypoint_source_node: Option<DebugSourceNodeId>,
    ) -> Self {
        self
    }

    fn execute<H: Host + Send>(
        self,
        program: &Program,
        host: &mut H,
    ) -> impl FutureMaybeSend<Result<ExecutionOutput, ExecutionError>> {
        async move {
            let Self { mut processor, debug_info } = self;
            let ctx = processor.get_initial_resume_context(program)?;
            let mut stepper = Stepper::new(processor, ctx);
            if let Some(debug_info) = debug_info {
                stepper = stepper.with_debug_info(debug_info);
            }
            let mut collector = Collector::new();
            let result = stepper.run(host, |cycle| collector.on_cycle(cycle)).await;
            #[cfg(feature = "std")]
            write_run(collector, &result);
            result?;
            stepper.into_output()
        }
    }
}

/// Writes the run to the coverage directory named by the environment, if any.
#[cfg(feature = "std")]
fn write_run(collector: Collector, result: &Result<(), ExecutionError>) {
    use alloc::string::ToString;

    use crate::{dir::CoverageDir, format::RunOutcome};

    let Some(dir) = CoverageDir::from_env() else {
        return;
    };
    let name = std::thread::current().name().unwrap_or("unnamed").to_string();
    let outcome = match result {
        Ok(()) => RunOutcome::Ok,
        Err(error) => RunOutcome::Failed(error.to_string()),
    };
    let output = collector.finish(name, outcome);
    if let Err(error) = dir.write_output(&output) {
        std::eprintln!(
            "miden-cov: failed to write run '{}' to '{}': {error}",
            output.run.name,
            dir.root().display()
        );
    }
}
