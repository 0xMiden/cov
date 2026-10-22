//! The coverage directory: where runs are written and the reporter reads.

use std::{
    format, fs, io,
    path::{Path, PathBuf},
    string::String,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
    vec::Vec,
};

use miden_core::serde::{Deserializable, Serializable};

use crate::{
    collector::CollectorOutput,
    format::{PACKAGE_FILE_EXTENSION, PackageFile, RUN_FILE_EXTENSION, RunFile},
};

/// Environment variable naming the coverage directory. When unset, in-test
/// collection stays inactive.
pub const COVERAGE_DIR_ENV: &str = "MIDEN_COV_DIR";

/// Default coverage directory relative to the working directory.
pub const DEFAULT_COVERAGE_DIR: &str = "target/miden-cov";

/// A coverage directory on disk.
///
/// Layout: `packages/<commitment>.pkg` written once per package and
/// `runs/<id>.run` written once per VM execution. Both kinds are written to a
/// temporary file and renamed into place, so a reader never sees a partial
/// file, and concurrent writers of the same package file are harmless.
#[derive(Debug, Clone)]
pub struct CoverageDir {
    root: PathBuf,
}

impl CoverageDir {
    /// Uses `root` as the coverage directory. Nothing is created until the
    /// first write.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The directory named by the environment variable, if set.
    pub fn from_env() -> Option<Self> {
        std::env::var_os(COVERAGE_DIR_ENV).map(Self::new)
    }

    /// The directory named by the environment variable, or the default.
    pub fn from_env_or_default() -> Self {
        Self::from_env().unwrap_or_else(|| Self::new(DEFAULT_COVERAGE_DIR))
    }

    /// Root of the directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where package files live.
    pub fn packages_dir(&self) -> PathBuf {
        self.root.join("packages")
    }

    /// Where run files live.
    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    /// Writes everything a collector produced: each package file unless it
    /// already exists, then the run file. Returns the run file path.
    pub fn write_output(&self, output: &CollectorOutput) -> io::Result<PathBuf> {
        for package in &output.packages {
            self.write_package(package)?;
        }
        self.write_run(&output.run)
    }

    /// Writes a package file unless one with the same commitment exists.
    pub fn write_package(&self, package: &PackageFile) -> io::Result<PathBuf> {
        let dir = self.packages_dir();
        let path = dir.join(package.file_name());
        if path.exists() {
            return Ok(path);
        }
        fs::create_dir_all(&dir)?;
        write_atomically(&path, &package.to_bytes())?;
        Ok(path)
    }

    /// Writes a run file under a fresh name and returns its path.
    pub fn write_run(&self, run: &RunFile) -> io::Result<PathBuf> {
        let dir = self.runs_dir();
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.{RUN_FILE_EXTENSION}", fresh_run_id()));
        write_atomically(&path, &run.to_bytes())?;
        Ok(path)
    }

    /// Reads every package file. Missing directory means no packages.
    pub fn read_packages(&self) -> io::Result<Vec<PackageFile>> {
        self.read_all(&self.packages_dir(), PACKAGE_FILE_EXTENSION)
    }

    /// Reads every run file. Missing directory means no runs.
    pub fn read_runs(&self) -> io::Result<Vec<RunFile>> {
        self.read_all(&self.runs_dir(), RUN_FILE_EXTENSION)
    }

    /// Reads one run file.
    pub fn read_run(path: &Path) -> io::Result<RunFile> {
        let bytes = fs::read(path)?;
        RunFile::read_from_bytes(&bytes).map_err(|e| invalid(path, e))
    }

    fn read_all<T: Deserializable>(&self, dir: &Path, extension: &str) -> io::Result<Vec<T>> {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == extension))
            .collect();
        paths.sort();
        paths
            .iter()
            .map(|path| {
                let bytes = fs::read(path)?;
                T::read_from_bytes(&bytes).map_err(|e| invalid(path, e))
            })
            .collect()
    }
}

fn invalid(path: &Path, error: impl core::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{}: {error}", path.display()))
}

/// Writes `bytes` to a temporary file next to `path` and renames it into
/// place.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", fresh_run_id()));
    fs::write(&tmp, bytes)?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        },
    }
}

/// A name that is unique across processes and across calls within a process.
fn fresh_run_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:024x}-{:08x}-{counter:04x}", std::process::id())
}
