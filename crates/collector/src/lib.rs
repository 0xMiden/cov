//! Collects code coverage by observing Miden VM program execution.
//!
//! The collector records, per package, how many times each source node
//! started and how many times each operation of each basic block executed.
//! It does not interpret those counts; the coverable rule that turns them into
//! line, function, and branch coverage lives in the reporter. The two sides
//! exchange data through the coverage directory (see [`dir`]), whose file
//! formats are defined in [`format`].
//!
//! The observer works on the processor's resume context, so it can be driven
//! by any loop that steps the VM one cycle at a time: the bundled
//! [`run_to_end`] driver, the debugger engine's executor, or a program
//! executor plugged into the protocol's transaction executor.

#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "std")]
pub mod dir;
pub mod format;
pub mod hits;
pub mod observer;

pub use format::{PackageFile, RunFile, RunOutcome};
pub use hits::PackageHits;
pub use miden_processor::{ExecutionError, FastProcessor, ResumeContext, SyncHost};
pub use observer::{Observer, ObserverOutput};

/// Steps `processor` from `ctx` until the program terminates, feeding every
/// cycle to `observer` first.
///
/// Returns the execution error if the program fails; the observer keeps the
/// hits recorded up to that point, which is what a failing test should report.
pub fn run_to_end(
    processor: &mut FastProcessor,
    host: &mut impl SyncHost,
    mut ctx: ResumeContext,
    observer: &mut Observer,
) -> Result<(), ExecutionError> {
    loop {
        observer.observe(&ctx);
        let next = match ctx.debug_info() {
            Some(debug_info) => {
                processor.step_with_package_debug_info_sync(host, ctx, &debug_info)?
            },
            None => processor.step_sync(host, ctx)?,
        };
        match next {
            Some(next) => ctx = next,
            None => return Ok(()),
        }
    }
}
