//! Collects code coverage of Miden VM program executions.
//!
//! The collector records, per package, how many times each source node
//! started, from which parent, and how many times each operation of each
//! basic block executed. It does not interpret those counts; the coverable
//! rule that turns them into line, function, and branch coverage lives in the
//! reporter. The two sides exchange data through the coverage directory (see
//! [`dir`]), whose file formats are defined in [`format`].
//!
//! The debugger engine resolves every VM cycle to source and hands the
//! collector one record per cycle. Any loop that steps the engine can feed a
//! [`Collector`]: the `miden-cov run` command drives the engine's stepper
//! with the host the engine builds, and [`CoverageExecutor`] drives it from
//! inside an executor generic over the VM's program executor, such as the
//! protocol's transaction executor, with that executor's host.

#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod collector;
#[cfg(feature = "std")]
pub mod dir;
pub mod executor;
pub mod format;
pub mod hits;

pub use collector::{Collector, CollectorOutput};
pub use executor::CoverageExecutor;
pub use format::{PackageFile, RunFile, RunOutcome};
pub use hits::PackageHits;
