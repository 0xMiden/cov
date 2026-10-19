//! Maps collected Miden VM coverage to source locations and renders coverage reports.
//!
//! The input is a coverage directory written by the collector. [`Snapshot`]
//! loads it and merges the runs, [`build_report`] applies the coverable rule
//! with source files found by a [`SourceResolver`], and the [`lcov`],
//! [`summary`], and [`dump`] modules render the result. Nothing here executes a
//! program.

use std::path::PathBuf;

pub mod dump;
pub mod lcov;
pub mod model;
pub mod snapshot;
pub mod source;
pub mod summary;

pub use model::{BranchCoverage, FileCoverage, FunctionCoverage, Report, build_report};
pub use snapshot::Snapshot;
pub use source::SourceResolver;

/// Errors raised while loading a coverage directory.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A directory or file of the coverage directory could not be read.
    #[error("cannot read coverage directory `{}`", dir.display())]
    Io {
        /// Root of the coverage directory.
        dir: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// A file of the coverage directory is not a coverage file this build can
    /// read; the message names the file and, for a version mismatch, the version.
    #[error("invalid coverage file: {0}")]
    Deserialization(String),
    /// A run refers to a package whose package file is missing.
    #[error(
        "run `{run}` refers to package {commitment}, which has no package file in the coverage \
         directory"
    )]
    MissingPackage {
        /// Name of the run.
        run: String,
        /// Commitment of the missing package, in hex.
        commitment: String,
    },
}
