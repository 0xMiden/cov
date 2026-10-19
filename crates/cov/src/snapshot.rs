//! The contents of a coverage directory, with the runs merged.

use std::{collections::BTreeMap, io};

use miden_core::Word;
use miden_cov_collector::{PackageFile, PackageHits, RunFile, dir::CoverageDir};

use crate::Error;

/// Every package and run of a coverage directory, plus the hits of all runs
/// summed per package.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Package files keyed by forest commitment.
    pub packages: BTreeMap<Word, PackageFile>,
    /// Every run, in file-name order.
    pub runs: Vec<RunFile>,
    /// The sum of all runs' hits, keyed by package commitment.
    pub merged: BTreeMap<Word, PackageHits>,
}

impl Snapshot {
    /// Reads and merges the coverage directory.
    ///
    /// Fails when a file cannot be read or decoded, or when a run refers to a
    /// package that has no package file.
    pub fn load(dir: &CoverageDir) -> Result<Self, Error> {
        let packages: BTreeMap<Word, PackageFile> = dir
            .read_packages()
            .map_err(|e| io_error(dir, e))?
            .into_iter()
            .map(|package| (package.commitment, package))
            .collect();
        let runs = dir.read_runs().map_err(|e| io_error(dir, e))?;

        let mut merged: BTreeMap<Word, PackageHits> = BTreeMap::new();
        for run in &runs {
            for (commitment, hits) in &run.packages {
                if !packages.contains_key(commitment) {
                    return Err(Error::MissingPackage {
                        run: run.name.clone(),
                        commitment: commitment.to_hex(),
                    });
                }
                merged.entry(*commitment).or_default().merge_from(hits);
            }
        }
        Ok(Self { packages, runs, merged })
    }
}

/// Classifies a coverage directory read error. The collector reports a file
/// it cannot decode as `InvalidData` with the path and reason in the message.
fn io_error(dir: &CoverageDir, error: io::Error) -> Error {
    if error.kind() == io::ErrorKind::InvalidData {
        Error::Deserialization(error.to_string())
    } else {
        Error::Io {
            dir: dir.root().to_path_buf(),
            source: error,
        }
    }
}
